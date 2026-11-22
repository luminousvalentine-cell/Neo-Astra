use serde_json::Value;

use crate::error::NopmError;
use crate::http::{HttpClient, encode_uri_component, get_json};
use crate::package_id::{PackageId, format_package_id, parse_dependency_request, parse_package_id};
use crate::resolve::Descriptor;

use super::{AdapterResolution, insert_dependency};

const PYPI: &str = "https://pypi.org";

pub fn resolve_pypi_package(
    id: &PackageId,
    alias: &str,
    client: &dyn HttpClient,
    registry: Option<&str>,
) -> Result<AdapterResolution, NopmError> {
    if id.source != crate::package_id::Source::Pypi {
        return Err(NopmError::with_path(
            "NP0011",
            "Expected a PyPI package ID.",
            format_package_id(id),
        ));
    }

    let registry = registry.unwrap_or(PYPI);
    let version = match &id.version {
        Some(version) => version.clone(),
        None => latest_version(client, registry, &id.name)?,
    };
    let url = format!(
        "{registry}/pypi/{}/{}/json",
        encode_uri_component(&id.name),
        encode_uri_component(&version)
    );
    let value = get_json(
        client,
        &url,
        &format!("read PyPI metadata for '{}@{version}'", id.name),
    )?;
    let info = value
        .get("info")
        .and_then(Value::as_object)
        .ok_or_else(|| {
            NopmError::with_path(
                "NP0011",
                format!("PyPI did not return info for '{}@{version}'.", id.name),
                url.clone(),
            )
        })?;

    let returned_name = string_field(info, "name").unwrap_or_else(|| id.name.clone());
    let returned_version = string_field(info, "version");
    if returned_version.as_deref() != Some(&version) {
        return Err(NopmError::with_path(
            "NP0011",
            format!(
                "PyPI returned version '{}' for '{}@{version}'.",
                returned_version.unwrap_or_else(|| "<missing>".to_owned()),
                id.name
            ),
            url.clone(),
        ));
    }

    let files: Vec<&serde_json::Map<String, Value>> = value
        .get("urls")
        .and_then(Value::as_array)
        .map(|files| files.iter().filter_map(Value::as_object).collect())
        .unwrap_or_default();
    let artifact = files
        .iter()
        .find(|entry| entry.get("packagetype").and_then(Value::as_str) == Some("sdist"))
        .or_else(|| {
            files.iter().find(|entry| {
                entry.get("packagetype").and_then(Value::as_str) == Some("bdist_wheel")
            })
        })
        .or_else(|| files.first())
        .ok_or_else(|| {
            NopmError::with_path(
                "NP0011",
                format!("PyPI has no distribution for '{}@{version}'.", id.name),
                url.clone(),
            )
        })?;
    let resolved = string_field(artifact, "url");
    let sha256 = artifact
        .get("digests")
        .and_then(Value::as_object)
        .and_then(|digests| string_field(digests, "sha256"));
    let (Some(resolved), Some(sha256)) = (resolved, sha256) else {
        return Err(NopmError::with_path(
            "NP0011",
            format!(
                "PyPI distribution for '{}@{version}' is missing a URL or sha256.",
                id.name
            ),
            url.clone(),
        ));
    };

    let mut dependencies = Vec::new();
    if let Some(requirements) = info.get("requires_dist").and_then(Value::as_array) {
        for requirement in requirements {
            if let Some(requirement) = requirement.as_str()
                && let Some((name, spec)) = parse_requirement(requirement)
            {
                insert_dependency(&mut dependencies, name, spec);
            }
        }
    }

    let exact_id = parse_package_id(&format!("pypi:{returned_name}@{version}"))?;
    let descriptor = Descriptor {
        id: exact_id.clone(),
        dependencies,
        targets: Vec::new(),
        capabilities: Vec::new(),
        resolved: Some(resolved),
        registry: Some(registry.to_owned()),
        integrity: Some(format!("sha256:{sha256}")),
        commit: None,
        lifecycle_scripts: false,
    };
    let request = parse_dependency_request(alias, &format_package_id(&exact_id))?;
    Ok(AdapterResolution {
        descriptor,
        request,
    })
}

fn latest_version(
    client: &dyn HttpClient,
    registry: &str,
    name: &str,
) -> Result<String, NopmError> {
    let url = format!("{registry}/pypi/{}/json", encode_uri_component(name));
    let value = get_json(client, &url, &format!("read PyPI metadata for '{name}'"))?;
    let version = value
        .get("info")
        .and_then(Value::as_object)
        .and_then(|info| string_field(info, "version"));
    version.ok_or_else(|| {
        NopmError::with_path(
            "NP0011",
            format!("PyPI did not return a latest version for '{name}'."),
            url,
        )
    })
}

fn parse_requirement(requirement: &str) -> Option<(String, String)> {
    let without_marker = requirement.split(';').next().unwrap_or("").trim();
    let (name, requirement_text) = split_requirement(without_marker)?;
    let name = name.replace('_', "-").to_lowercase();
    if let Some(exact) = exact_requirement(requirement_text) {
        return Some((name.clone(), format!("pypi:{name}@{exact}")));
    }
    if requirement_text.trim().is_empty() {
        return Some((name.clone(), format!("pypi:{name}")));
    }
    None
}

fn split_requirement(value: &str) -> Option<(&str, &str)> {
    let name_end = value
        .find(|character: char| {
            !(character.is_ascii_alphanumeric() || matches!(character, '.' | '_' | '-'))
        })
        .unwrap_or(value.len());
    if name_end == 0 {
        return None;
    }
    Some((&value[..name_end], value[name_end..].trim_start()))
}

fn exact_requirement(requirement: &str) -> Option<String> {
    let mut rest = requirement;
    while let Some(index) = rest.find("==") {
        let before = rest[..index].chars().last();
        if before.is_none() || before == Some(',') || before == Some(' ') {
            let candidate: String = rest[index + 2..]
                .trim_start()
                .chars()
                .take_while(|character| {
                    character.is_ascii_alphanumeric()
                        || matches!(character, '.' | '_' | '+' | '!' | '-')
                })
                .collect();
            if !candidate.is_empty() {
                return Some(candidate);
            }
        }
        rest = &rest[index + 2..];
    }
    None
}

fn string_field(value: &serde_json::Map<String, Value>, key: &str) -> Option<String> {
    value
        .get(key)
        .and_then(Value::as_str)
        .filter(|entry| !entry.is_empty())
        .map(str::to_owned)
}
