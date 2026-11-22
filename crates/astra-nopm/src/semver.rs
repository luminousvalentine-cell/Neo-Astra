use crate::error::NopmError;
use crate::package_id::locale_cmp;

type Matcher = Box<dyn Fn(&SemVer) -> bool>;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SemVer {
    pub major: u64,
    pub minor: u64,
    pub patch: u64,
    pub pre: Option<String>,
}

pub fn is_exact_semver(text: &str) -> bool {
    let (without_build, build) = match text.split_once('+') {
        Some((left, right)) => (left, Some(right)),
        None => (text, None),
    };
    let (core, prerelease) = match without_build.split_once('-') {
        Some((left, right)) => (left, Some(right)),
        None => (without_build, None),
    };
    let parts: Vec<&str> = core.split('.').collect();
    if parts.len() != 3 || !parts.iter().all(|part| is_numeric_identifier(part)) {
        return false;
    }
    if let Some(prerelease) = prerelease
        && !prerelease.split('.').all(is_identifier)
    {
        return false;
    }
    if let Some(build) = build
        && !build.split('.').all(is_identifier)
    {
        return false;
    }
    true
}

pub fn parse_semver(text: &str) -> Option<SemVer> {
    let (without_build, build) = match text.split_once('+') {
        Some((left, right)) => (left, Some(right)),
        None => (text, None),
    };
    let (core, prerelease) = match without_build.split_once('-') {
        Some((left, right)) => (left, Some(right)),
        None => (without_build, None),
    };
    let parts: Vec<&str> = core.split('.').collect();
    if parts.is_empty() || parts.len() > 3 {
        return None;
    }
    if !parts.iter().all(|part| is_numeric_identifier(part)) {
        return None;
    }
    if let Some(prerelease) = prerelease
        && (prerelease.is_empty()
            || !prerelease
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '.' || c == '-'))
    {
        return None;
    }
    if let Some(build) = build
        && (build.is_empty()
            || !build
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '.' || c == '-'))
    {
        return None;
    }

    Some(SemVer {
        major: parts[0].parse().ok()?,
        minor: parts.get(1).and_then(|part| part.parse().ok()).unwrap_or(0),
        patch: parts.get(2).and_then(|part| part.parse().ok()).unwrap_or(0),
        pre: prerelease.map(str::to_owned),
    })
}

fn is_numeric_identifier(text: &str) -> bool {
    !text.is_empty()
        && text.chars().all(|character| character.is_ascii_digit())
        && (text == "0" || !text.starts_with('0'))
}

fn is_identifier(text: &str) -> bool {
    !text.is_empty()
        && text
            .chars()
            .all(|character| character.is_ascii_alphanumeric() || character == '-')
}

pub fn compare_semver(left: &SemVer, right: &SemVer) -> std::cmp::Ordering {
    match left.major.cmp(&right.major) {
        std::cmp::Ordering::Equal => {}
        ordering => return ordering,
    }
    match left.minor.cmp(&right.minor) {
        std::cmp::Ordering::Equal => {}
        ordering => return ordering,
    }
    match left.patch.cmp(&right.patch) {
        std::cmp::Ordering::Equal => {}
        ordering => return ordering,
    }
    match (&left.pre, &right.pre) {
        (None, None) => std::cmp::Ordering::Equal,
        (None, Some(_)) => std::cmp::Ordering::Greater,
        (Some(_), None) => std::cmp::Ordering::Less,
        (Some(left), Some(right)) => locale_cmp(left, right),
    }
}

pub fn requirement_matcher(requirement: &str) -> Result<Matcher, NopmError> {
    let parts: Vec<&str> = requirement
        .split(|character: char| character == ',' || character.is_whitespace())
        .map(str::trim)
        .filter(|part| !part.is_empty())
        .collect();
    let matchers: Vec<Matcher> = parts
        .into_iter()
        .map(comparator_matcher)
        .collect::<Result<_, _>>()?;
    let allow_prerelease = requirement.contains('-');
    Ok(Box::new(move |version: &SemVer| {
        if version.pre.is_some() && !allow_prerelease {
            return false;
        }
        matchers.iter().all(|matcher| matcher(version))
    }))
}

fn comparator_matcher(comparator: &str) -> Result<Matcher, NopmError> {
    if comparator == "*" {
        return Ok(Box::new(|_| true));
    }
    for operator in [">=", "<=", ">", "<", "="] {
        if let Some(rest) = comparator.strip_prefix(operator) {
            let version = parse_required_version(rest.trim())?;
            let operator = operator.to_owned();
            return Ok(Box::new(move |candidate: &SemVer| {
                let ordering = compare_semver(candidate, &version);
                match operator.as_str() {
                    ">=" => ordering != std::cmp::Ordering::Less,
                    "<=" => ordering != std::cmp::Ordering::Greater,
                    ">" => ordering == std::cmp::Ordering::Greater,
                    "<" => ordering == std::cmp::Ordering::Less,
                    _ => ordering == std::cmp::Ordering::Equal,
                }
            }));
        }
    }
    if let Some(rest) = comparator.strip_prefix('^') {
        return Ok(caret_matcher(parse_required_version(rest)?));
    }
    if let Some(rest) = comparator.strip_prefix('~') {
        return Ok(tilde_matcher(parse_required_version(rest)?));
    }
    if comparator.contains('*') {
        return wildcard_matcher(comparator);
    }
    Ok(caret_matcher(parse_required_version(comparator)?))
}

fn parse_required_version(text: &str) -> Result<SemVer, NopmError> {
    parse_semver(text).ok_or_else(|| {
        NopmError::with_path(
            "NP0011",
            format!("Invalid Cargo version requirement '{text}'."),
            text,
        )
    })
}

fn caret_matcher(version: SemVer) -> Box<dyn Fn(&SemVer) -> bool> {
    let upper = if version.major > 0 {
        SemVer {
            major: version.major + 1,
            minor: 0,
            patch: 0,
            pre: None,
        }
    } else if version.minor > 0 {
        SemVer {
            major: 0,
            minor: version.minor + 1,
            patch: 0,
            pre: None,
        }
    } else {
        SemVer {
            major: 0,
            minor: 0,
            patch: version.patch + 1,
            pre: None,
        }
    };
    Box::new(move |candidate: &SemVer| {
        compare_semver(candidate, &version) != std::cmp::Ordering::Less
            && compare_semver(candidate, &upper) == std::cmp::Ordering::Less
    })
}

fn tilde_matcher(version: SemVer) -> Box<dyn Fn(&SemVer) -> bool> {
    let upper = if version.major > 0 || version.minor == 0 {
        SemVer {
            major: version.major + 1,
            minor: 0,
            patch: 0,
            pre: None,
        }
    } else {
        SemVer {
            major: 0,
            minor: version.minor + 1,
            patch: 0,
            pre: None,
        }
    };
    Box::new(move |candidate: &SemVer| {
        compare_semver(candidate, &version) != std::cmp::Ordering::Less
            && compare_semver(candidate, &upper) == std::cmp::Ordering::Less
    })
}

fn wildcard_matcher(requirement: &str) -> Result<Matcher, NopmError> {
    let mut parts = requirement.split('.');
    let major_text = parts.next().unwrap_or("");
    let minor_text = parts.next();
    let major: u64 = major_text.parse().map_err(|_| {
        NopmError::with_path(
            "NP0011",
            format!("Invalid Cargo wildcard '{requirement}'."),
            requirement,
        )
    })?;
    if minor_text == Some("*") || minor_text.is_none() {
        return Ok(Box::new(move |candidate: &SemVer| candidate.major == major));
    }
    let minor: u64 = minor_text.unwrap_or("").parse().map_err(|_| {
        NopmError::with_path(
            "NP0011",
            format!("Invalid Cargo wildcard '{requirement}'."),
            requirement,
        )
    })?;
    Ok(Box::new(move |candidate: &SemVer| {
        candidate.major == major && candidate.minor == minor
    }))
}
