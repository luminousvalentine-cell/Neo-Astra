use serde_json::Value;

use crate::error::NopmError;
use crate::http::{HttpClient, encode_uri_component, get_json};
use crate::package_id::{PackageId, format_package_id, parse_dependency_request, parse_package_id};
use crate::resolve::Descriptor;
use crate::semver::{compare_semver, is_exact_semver, parse_semver, requirement_matcher};

use super::{AdapterResolution, insert_dependency};

const CRATES_IO: &str = "https://crates.io";

pub fn resolve_cargo_package(
    id: &PackageId,
    alias: &str,
    client: &dyn HttpClient,
    registry: Option<&str>,
) -> Result<AdapterResolution, NopmError> {
    if id.source != crate::package_id::Source::Cargo {
        return Err(NopmError::with_path(
            "NP0011",
            "Expected a Cargo package ID.",
            format_package_id(id),
        ));
    }

    let registry = registry.unwrap_or(CRATES_IO);
    let mut version = id.version.clone();
    if version
        .as_deref()
        .map(|version| !is_exact_semver(version))
        .unwrap_or(true)
    {
        let crate_metadata = get_json(
            client,
            &format!(
                "{registry}/api/v1/crates/{}",
                encode_uri_component(&id.name)
            ),
            &format!("read crates.io metadata for '{}'", id.name),
        )?;
        version = Some(resolve_cargo_version(
            &id.name,
            version.as_deref(),
            &crate_metadata,
        )?);
    }
    let version = version.expect("version resolved");

    let details = get_json(
        client,
        &format!(
            "{registry}/api/v1/crates/{}/{}",
            encode_uri_component(&id.name),
            encode_uri_component(&version)
        ),
        &format!("read crates.io metadata for '{}@{version}'", id.name),
    )?;
    let version_record = details
        .get("version")
        .and_then(Value::as_object)
        .cloned()
        .unwrap_or_else(|| details.as_object().cloned().unwrap_or_default());
    let returned_version = string_field(&version_record, "num");
    if returned_version.as_deref() != Some(&version) {
        return Err(NopmError::with_path(
            "NP0011",
            format!(
                "crates.io returned version '{}' for '{}@{version}'.",
                returned_version.unwrap_or_else(|| "<missing>".to_owned()),
                id.name
            ),
            format!("{}@{version}", id.name),
        ));
    }
    if version_record.get("yanked").and_then(Value::as_bool) == Some(true) {
        return Err(NopmError::with_path(
            "NP0011",
            format!("Cargo package '{}@{version}' is yanked.", id.name),
            format!("{}@{version}", id.name),
        ));
    }

    let checksum = string_field(&version_record, "checksum").ok_or_else(|| {
        NopmError::with_path(
            "NP0011",
            format!(
                "crates.io metadata for '{}@{version}' is missing a checksum.",
                id.name
            ),
            format!("{}@{version}", id.name),
        )
    })?;

    let dependencies_value = get_json(
        client,
        &format!(
            "{registry}/api/v1/crates/{}/{}/dependencies",
            encode_uri_component(&id.name),
            encode_uri_component(&version)
        ),
        &format!("read crates.io dependencies for '{}@{version}'", id.name),
    )?;
    let mut dependencies = Vec::new();
    if let Some(entries) = dependencies_value
        .get("dependencies")
        .and_then(Value::as_array)
    {
        for dependency in entries {
            let Some(dependency) = dependency.as_object() else {
                continue;
            };
            let kind = string_field(dependency, "kind").unwrap_or_else(|| "normal".to_owned());
            if kind != "normal" || dependency.get("optional").and_then(Value::as_bool) == Some(true)
            {
                continue;
            }
            let name = string_field(dependency, "package")
                .or_else(|| string_field(dependency, "crate_id"));
            let requirement = string_field(dependency, "req");
            if let (Some(name), Some(requirement)) = (name, requirement) {
                insert_dependency(&mut dependencies, name, requirement);
            }
        }
    }

    let exact_id = parse_package_id(&format!("cargo:{}@{version}", id.name))?;
    let descriptor = Descriptor {
        id: exact_id.clone(),
        dependencies,
        targets: Vec::new(),
        capabilities: Vec::new(),
        resolved: Some(format!(
            "https://static.crates.io/crates/{}/{}-{version}.crate",
            id.name, id.name
        )),
        registry: Some(registry.to_owned()),
        integrity: Some(format!("sha256:{checksum}")),
        commit: None,
        lifecycle_scripts: false,
    };
    let request = parse_dependency_request(alias, &format_package_id(&exact_id))?;
    Ok(AdapterResolution {
        descriptor,
        request,
    })
}

fn resolve_cargo_version(
    name: &str,
    requirement: Option<&str>,
    crate_metadata: &Value,
) -> Result<String, NopmError> {
    let versions = crate_metadata
        .get("versions")
        .and_then(Value::as_array)
        .map(|versions| {
            versions
                .iter()
                .filter_map(Value::as_object)
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    let mut candidates: Vec<(String, crate::semver::SemVer)> = Vec::new();
    for entry in versions {
        let Some(text) = string_field(entry, "num") else {
            continue;
        };
        if entry.get("yanked").and_then(Value::as_bool) == Some(true) {
            continue;
        }
        if let Some(version) = parse_semver(&text) {
            candidates.push((text, version));
        }
    }

    if requirement.is_none() {
        let crate_record = crate_metadata
            .get("crate")
            .and_then(Value::as_object)
            .cloned()
            .unwrap_or_else(|| crate_metadata.as_object().cloned().unwrap_or_default());
        if let Some(max_stable) = string_field(&crate_record, "max_stable_version") {
            return Ok(max_stable);
        }
        candidates.sort_by(|left, right| compare_semver(&right.1, &left.1));
        if let Some((text, _)) = candidates
            .iter()
            .find(|(_, version)| version.pre.is_none())
            .or_else(|| candidates.first())
        {
            return Ok(text.clone());
        }
        return Err(NopmError::with_path(
            "NP0011",
            format!("crates.io has no usable versions for '{name}'."),
            name,
        ));
    }

    let requirement = requirement.expect("requirement");
    let matcher = requirement_matcher(requirement)?;
    candidates.sort_by(|left, right| compare_semver(&right.1, &left.1));
    if let Some((text, _)) = candidates.iter().find(|(_, version)| matcher(version)) {
        return Ok(text.clone());
    }
    Err(NopmError::with_path(
        "NP0011",
        format!("No crates.io version of '{name}' satisfies '{requirement}'."),
        format!("{name}@{requirement}"),
    ))
}

fn string_field(value: &serde_json::Map<String, Value>, key: &str) -> Option<String> {
    value
        .get(key)
        .and_then(Value::as_str)
        .filter(|entry| !entry.is_empty())
        .map(str::to_owned)
}
