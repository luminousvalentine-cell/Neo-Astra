use serde_json::Value;

use crate::error::NopmError;
use crate::http::{HttpClient, encode_uri_component, get_json};
use crate::package_id::{PackageId, format_package_id, parse_dependency_request, parse_package_id};
use crate::resolve::Descriptor;

use super::{AdapterResolution, insert_dependency};

const NUGET: &str = "https://api.nuget.org";

pub fn resolve_nuget_package(
    id: &PackageId,
    alias: &str,
    client: &dyn HttpClient,
    registry: Option<&str>,
) -> Result<AdapterResolution, NopmError> {
    if id.source != crate::package_id::Source::Nuget {
        return Err(NopmError::with_path(
            "NP0011",
            "Expected a NuGet package ID.",
            format_package_id(id),
        ));
    }

    let registry = registry.unwrap_or(NUGET);
    let lower = id.name.to_lowercase();
    let version = match &id.version {
        Some(version) => version.clone(),
        None => latest_version(client, registry, &lower)?,
    };
    let nupkg = format!(
        "{registry}/v3-flatcontainer/{}/{}/{}.{}.nupkg",
        encode_uri_component(&lower),
        encode_uri_component(&version),
        encode_uri_component(&lower),
        encode_uri_component(&version)
    );

    let integrity = fetch_integrity(client, &format!("{nupkg}.sha512"))?;
    let dependencies = fetch_dependencies(client, registry, &lower, &version)?;
    let exact_id = parse_package_id(&format!("nuget:{}@{version}", id.name))?;
    let descriptor = Descriptor {
        id: exact_id.clone(),
        dependencies,
        targets: Vec::new(),
        capabilities: Vec::new(),
        resolved: Some(nupkg),
        registry: Some(registry.to_owned()),
        integrity: Some(integrity),
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
    lower: &str,
) -> Result<String, NopmError> {
    let value = get_json(
        client,
        &format!(
            "{registry}/v3-flatcontainer/{}/index.json",
            encode_uri_component(lower)
        ),
        &format!("read NuGet versions for '{lower}'"),
    )?;
    let stable = value
        .get("versions")
        .and_then(Value::as_array)
        .and_then(|versions| {
            versions
                .iter()
                .filter_map(Value::as_str)
                .rfind(|version| !version.contains('-'))
        });
    stable.map(str::to_owned).ok_or_else(|| {
        NopmError::with_path(
            "NP0011",
            format!("NuGet did not return a stable version for '{lower}'."),
            lower,
        )
    })
}

fn fetch_integrity(client: &dyn HttpClient, url: &str) -> Result<String, NopmError> {
    let response = client
        .get(url, &[("Accept", "application/json")])
        .map_err(|message| {
            NopmError::with_path(
                "NP0011",
                format!("Unable to read NuGet package integrity: {message}"),
                url,
            )
        })?;
    if !response.ok() {
        return Err(NopmError::with_path(
            "NP0011",
            format!("Unable to read NuGet integrity: HTTP {}.", response.status),
            url,
        ));
    }
    let text = response.text();
    let text = text.trim();
    if text.is_empty() {
        return Err(NopmError::with_path(
            "NP0011",
            "NuGet returned an empty integrity hash.",
            url,
        ));
    }
    Ok(format!("sha512-{text}"))
}

fn fetch_dependencies(
    client: &dyn HttpClient,
    registry: &str,
    lower: &str,
    version: &str,
) -> Result<Vec<(String, String)>, NopmError> {
    let value = get_json(
        client,
        &format!(
            "{registry}/v3/registration5-gz-semver2/{}/{}.json",
            encode_uri_component(lower),
            encode_uri_component(version)
        ),
        &format!("read NuGet dependencies for '{lower}@{version}'"),
    )?;
    let catalog = value
        .get("catalogEntry")
        .and_then(Value::as_object)
        .cloned()
        .unwrap_or_else(|| value.as_object().cloned().unwrap_or_default());
    let mut dependencies = Vec::new();
    if let Some(groups) = catalog.get("dependencyGroups").and_then(Value::as_array) {
        for group in groups {
            let Some(group) = group.as_object() else {
                continue;
            };
            let Some(entries) = group.get("dependencies").and_then(Value::as_array) else {
                continue;
            };
            for dependency in entries {
                let Some(dependency) = dependency.as_object() else {
                    continue;
                };
                let name = string_field(dependency, "id");
                let range = string_field(dependency, "range");
                if let (Some(name), Some(range)) = (name, range)
                    && let Some(exact) = exact_range(&range)
                {
                    insert_dependency(
                        &mut dependencies,
                        name.clone(),
                        format!("nuget:{name}@{exact}"),
                    );
                }
            }
        }
    }
    Ok(dependencies)
}

fn exact_range(range: &str) -> Option<String> {
    let inner = range.strip_prefix('[')?.strip_suffix(']')?;
    let value = inner.trim();
    if value.is_empty() || value.contains(',') || value.chars().any(char::is_whitespace) {
        return None;
    }
    Some(value.to_owned())
}

fn string_field(value: &serde_json::Map<String, Value>, key: &str) -> Option<String> {
    value
        .get(key)
        .and_then(Value::as_str)
        .filter(|entry| !entry.is_empty())
        .map(str::to_owned)
}
