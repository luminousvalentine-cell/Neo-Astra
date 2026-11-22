use crate::error::NopmError;

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Source {
    Nova,
    Npm,
    Cargo,
    Git,
    Path,
    Pypi,
    Nuget,
    Oci,
}

impl Source {
    pub fn as_str(self) -> &'static str {
        match self {
            Source::Nova => "nova",
            Source::Npm => "npm",
            Source::Cargo => "cargo",
            Source::Git => "git",
            Source::Path => "path",
            Source::Pypi => "pypi",
            Source::Nuget => "nuget",
            Source::Oci => "oci",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PackageId {
    pub source: Source,
    pub name: String,
    pub version: Option<String>,
    pub reference: Option<String>,
    pub path: Option<String>,
    pub raw: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DependencyRequest {
    pub alias: String,
    pub spec: String,
    pub id: PackageId,
}

fn invalid_id(input: &str, message: impl Into<String>) -> NopmError {
    NopmError::with_path("NP0001", message, input)
}

fn is_npm_name_char(c: char) -> bool {
    c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '~' | '-')
}

fn is_npm_name(name: &str) -> bool {
    !name.is_empty() && name.chars().all(is_npm_name_char)
}

fn is_scoped_name(name: &str) -> bool {
    let Some(rest) = name.strip_prefix('@') else {
        return false;
    };
    let mut parts = rest.split('/');
    let scope = parts.next().unwrap_or("");
    let package = parts.next().unwrap_or("");
    parts.next().is_none() && is_npm_name(scope) && is_npm_name(package)
}

fn is_cargo_name(name: &str) -> bool {
    !name.is_empty()
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '-'))
}

fn is_ecosystem_name(name: &str) -> bool {
    !name.is_empty()
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-'))
}

fn is_alias(alias: &str) -> bool {
    let mut chars = alias.chars();
    let Some(first) = chars.next() else {
        return false;
    };
    if !(first.is_ascii_alphabetic() || first == '_') {
        return false;
    }
    chars.all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '-'))
}

fn is_semver(value: &str) -> bool {
    let (without_build, build) = match value.split_once('+') {
        Some((left, right)) => (left, Some(right)),
        None => (value, None),
    };
    let (core, prerelease) = match without_build.split_once('-') {
        Some((left, right)) => (left, Some(right)),
        None => (without_build, None),
    };

    let parts: Vec<&str> = core.split('.').collect();
    if parts.len() != 3 {
        return false;
    }
    for part in parts {
        if part.is_empty() || !part.chars().all(|c| c.is_ascii_digit()) {
            return false;
        }
        if part.len() > 1 && part.starts_with('0') {
            return false;
        }
    }

    let valid_identifiers = |text: &str| {
        !text.is_empty()
            && text.split('.').all(|identifier| {
                !identifier.is_empty()
                    && identifier
                        .chars()
                        .all(|c| c.is_ascii_alphanumeric() || c == '-')
            })
    };

    if let Some(prerelease) = prerelease
        && !valid_identifiers(prerelease)
    {
        return false;
    }
    if let Some(build) = build
        && !valid_identifiers(build)
    {
        return false;
    }
    true
}

fn validate_scoped_or_plain_name(name: &str, raw: &str, source: &str) -> Result<(), NopmError> {
    if name.starts_with('@') {
        if !is_scoped_name(name) {
            return Err(invalid_id(
                raw,
                format!("Invalid {source} scoped package name '{name}'."),
            ));
        }
    } else if !is_npm_name(name) {
        return Err(invalid_id(
            raw,
            format!("Invalid {source} package name '{name}'."),
        ));
    }
    Ok(())
}

fn split_version(rest: &str, raw: &str) -> Result<(String, Option<String>), NopmError> {
    let scoped = rest.starts_with('@');
    let search_from = if scoped { 1 } else { 0 };
    let at = rest[search_from..]
        .find('@')
        .map(|index| index + search_from);
    if scoped && at.is_none() {
        return Err(invalid_id(
            raw,
            "Scoped package names must use @scope/name.",
        ));
    }
    let Some(at) = at else {
        return Ok((rest.to_owned(), None));
    };
    if scoped && at <= 1 {
        return Err(invalid_id(
            raw,
            "Scoped package names must use @scope/name.",
        ));
    }
    let name = rest[..at].to_owned();
    let version = rest[at + 1..].to_owned();
    if version.is_empty() {
        return Err(invalid_id(
            raw,
            "Package versions after '@' must not be empty.",
        ));
    }
    Ok((name, Some(version)))
}

fn validate_version_text(version: &str, raw: &str) -> Result<(), NopmError> {
    if version.is_empty() || version.trim() != version || version.contains('@') {
        return Err(invalid_id(
            raw,
            format!("Invalid package version or range '{version}'."),
        ));
    }
    Ok(())
}

fn make_id(
    source: Source,
    name: String,
    version: Option<String>,
    reference: Option<String>,
    path: Option<String>,
    raw: &str,
) -> PackageId {
    PackageId {
        source,
        name,
        version,
        reference,
        path,
        raw: raw.to_owned(),
    }
}

pub fn parse_package_id(input: &str) -> Result<PackageId, NopmError> {
    if input.is_empty() || input.trim() != input {
        return Err(invalid_id(
            input,
            "Package IDs must not be empty or contain surrounding whitespace.",
        ));
    }

    if let Some(rest) = input.strip_prefix("nova:") {
        let (name, version) = split_version(rest, input)?;
        validate_scoped_or_plain_name(&name, input, "nova")?;
        let Some(version) = version else {
            return Err(invalid_id(
                input,
                "Nova package IDs require an exact version, such as nova:name@1.2.3.",
            ));
        };
        if !is_semver(&version) {
            return Err(invalid_id(
                input,
                format!("Nova package version '{version}' is not an exact semantic version."),
            ));
        }
        return Ok(make_id(
            Source::Nova,
            name,
            Some(version),
            None,
            None,
            input,
        ));
    }
    if let Some(rest) = input.strip_prefix("npm:") {
        let (name, version) = split_version(rest, input)?;
        validate_scoped_or_plain_name(&name, input, "npm")?;
        if let Some(version) = &version {
            validate_version_text(version, input)?;
        }
        return Ok(make_id(Source::Npm, name, version, None, None, input));
    }
    if let Some(rest) = input.strip_prefix("cargo:") {
        let (name, version) = split_version(rest, input)?;
        if !is_cargo_name(&name) {
            return Err(invalid_id(
                input,
                format!("Invalid Cargo package name '{name}'."),
            ));
        }
        if let Some(version) = &version {
            validate_version_text(version, input)?;
        }
        return Ok(make_id(Source::Cargo, name, version, None, None, input));
    }
    if let Some(rest) = input.strip_prefix("git+") {
        let Some(hash) = rest.rfind('#') else {
            return Err(invalid_id(
                input,
                "Git package IDs require a URL and a pinned #reference.",
            ));
        };
        if hash == 0 || hash == rest.len() - 1 {
            return Err(invalid_id(
                input,
                "Git package IDs require a URL and a pinned #reference.",
            ));
        }
        let url = &rest[..hash];
        let reference = &rest[hash + 1..];
        let lower = url.to_ascii_lowercase();
        if !(lower.starts_with("http://")
            || lower.starts_with("https://")
            || lower.starts_with("ssh://"))
            || url.chars().any(char::is_whitespace)
            || url.contains('#')
        {
            return Err(invalid_id(
                input,
                "Git package URLs must use http, https, or ssh and contain no whitespace.",
            ));
        }
        if reference.is_empty()
            || reference.chars().any(char::is_whitespace)
            || reference.contains('#')
        {
            return Err(invalid_id(
                input,
                "Git references must be non-empty and contain no whitespace or '#'.",
            ));
        }
        return Ok(make_id(
            Source::Git,
            url.to_owned(),
            None,
            Some(reference.to_owned()),
            None,
            input,
        ));
    }
    if let Some(rest) = input.strip_prefix("path:") {
        if rest.is_empty() || rest.trim() != rest || rest.contains('\0') {
            return Err(invalid_id(
                input,
                "Path package IDs require a non-empty path without surrounding whitespace.",
            ));
        }
        let segments: Vec<&str> = rest
            .split(['\\', '/'])
            .filter(|segment| !segment.is_empty())
            .collect();
        let name = segments.last().copied().unwrap_or(rest);
        return Ok(make_id(
            Source::Path,
            name.to_owned(),
            None,
            None,
            Some(rest.to_owned()),
            input,
        ));
    }
    if let Some(rest) = input.strip_prefix("pypi:") {
        let (name, version) = split_version(rest, input)?;
        if !is_ecosystem_name(&name) {
            return Err(invalid_id(
                input,
                format!("Invalid pypi package name '{name}'."),
            ));
        }
        if let Some(version) = &version {
            validate_version_text(version, input)?;
        }
        return Ok(make_id(Source::Pypi, name, version, None, None, input));
    }
    if let Some(rest) = input.strip_prefix("nuget:") {
        let (name, version) = split_version(rest, input)?;
        if !is_ecosystem_name(&name) {
            return Err(invalid_id(
                input,
                format!("Invalid nuget package name '{name}'."),
            ));
        }
        if let Some(version) = &version {
            validate_version_text(version, input)?;
        }
        return Ok(make_id(Source::Nuget, name, version, None, None, input));
    }
    if let Some(rest) = input.strip_prefix("oci:") {
        let Some(separator) = rest.rfind('@') else {
            return Err(invalid_id(
                input,
                "OCI package IDs require registry/repository@reference.",
            ));
        };
        if separator == 0 || separator == rest.len() - 1 {
            return Err(invalid_id(
                input,
                "OCI package IDs require registry/repository@reference.",
            ));
        }
        let name = &rest[..separator];
        let reference = &rest[separator + 1..];
        let segments: Vec<&str> = name.split('/').collect();
        if segments.len() < 2 || segments.iter().any(|segment| segment.is_empty()) {
            return Err(invalid_id(
                input,
                "OCI package names require registry/repository.",
            ));
        }
        if name.chars().any(char::is_whitespace)
            || reference.chars().any(char::is_whitespace)
            || reference.contains('@')
        {
            return Err(invalid_id(
                input,
                "OCI names and references must not contain whitespace or '@'.",
            ));
        }
        return Ok(make_id(
            Source::Oci,
            name.to_owned(),
            None,
            Some(reference.to_owned()),
            None,
            input,
        ));
    }

    Err(invalid_id(
        input,
        "Package IDs must be source-qualified with nova:, npm:, cargo:, git+, path:, pypi:, nuget:, or oci:.",
    ))
}

pub fn format_package_id(id: &PackageId) -> String {
    match id.source {
        Source::Nova => format!("nova:{}@{}", id.name, id.version.as_deref().unwrap_or("")),
        Source::Npm => format!(
            "npm:{}{}",
            id.name,
            id.version
                .as_ref()
                .map(|version| format!("@{version}"))
                .unwrap_or_default()
        ),
        Source::Cargo => format!(
            "cargo:{}{}",
            id.name,
            id.version
                .as_ref()
                .map(|version| format!("@{version}"))
                .unwrap_or_default()
        ),
        Source::Git => format!("git+{}#{}", id.name, id.reference.as_deref().unwrap_or("")),
        Source::Path => format!("path:{}", id.path.as_deref().unwrap_or("")),
        Source::Pypi => format!(
            "pypi:{}{}",
            id.name,
            id.version
                .as_ref()
                .map(|version| format!("@{version}"))
                .unwrap_or_default()
        ),
        Source::Nuget => format!(
            "nuget:{}{}",
            id.name,
            id.version
                .as_ref()
                .map(|version| format!("@{version}"))
                .unwrap_or_default()
        ),
        Source::Oci => format!("oci:{}@{}", id.name, id.reference.as_deref().unwrap_or("")),
    }
}

pub fn parse_dependency_request(alias: &str, spec: &str) -> Result<DependencyRequest, NopmError> {
    if !is_alias(alias) {
        return Err(NopmError::with_path(
            "NP0002",
            format!(
                "Invalid dependency alias '{alias}'. Use letters, digits, '_' or '-', starting with a letter or '_'."
            ),
            alias,
        ));
    }

    let id = parse_package_id(spec).map_err(|error| {
        NopmError::with_path(
            "NP0002",
            format!("Invalid dependency '{alias}': {}", error.message),
            spec,
        )
    })?;

    Ok(DependencyRequest {
        alias: alias.to_owned(),
        spec: spec.to_owned(),
        id,
    })
}

pub fn locale_cmp(left: &str, right: &str) -> std::cmp::Ordering {
    const ORDER: &str = " _-,;:!?.'\"()[]{}@*/\\&#%`^+<=>|~$0123456789aAbBcCdDeEfFgGhHiIjJkKlLmMnNoOpPqQrRsStTuUvVwWxXyYzZ";

    fn rank(c: char) -> usize {
        ORDER
            .chars()
            .position(|candidate| candidate == c)
            .unwrap_or(ORDER.chars().count() + c as usize)
    }

    let mut left_chars = left.chars();
    let mut right_chars = right.chars();
    loop {
        match (left_chars.next(), right_chars.next()) {
            (None, None) => return std::cmp::Ordering::Equal,
            (None, Some(_)) => return std::cmp::Ordering::Less,
            (Some(_), None) => return std::cmp::Ordering::Greater,
            (Some(a), Some(b)) => match rank(a).cmp(&rank(b)) {
                std::cmp::Ordering::Equal => continue,
                other => return other,
            },
        }
    }
}
