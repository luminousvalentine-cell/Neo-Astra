use serde_json::Value;

use crate::command::{CommandRunner, command_name};
use crate::error::NopmError;
use crate::package_id::{PackageId, format_package_id, parse_dependency_request, parse_package_id};
use crate::resolve::Descriptor;

use super::AdapterResolution;

pub fn resolve_npm_package(
    id: &PackageId,
    alias: &str,
    runner: &dyn CommandRunner,
) -> Result<AdapterResolution, NopmError> {
    if id.source != crate::package_id::Source::Npm {
        return Err(NopmError::with_path(
            "NP0011",
            "Expected an npm package ID.",
            format_package_id(id),
        ));
    }

    let requested = match &id.version {
        Some(version) => format!("{}@{version}", id.name),
        None => id.name.clone(),
    };
    let version_json = run_npm(
        runner,
        &["view", &requested, "version", "--json"],
        &format!("resolve npm package '{requested}'"),
    )?;
    let version = selected_version(&version_json, &requested)?;
    let exact_name = format!("{}@{version}", id.name);
    let metadata_json = run_npm(
        runner,
        &["view", &exact_name, "--json"],
        &format!("read npm metadata for '{exact_name}'"),
    )?;
    let metadata = select_metadata(&metadata_json, &version, &exact_name)?;

    let name = string_field(&metadata, "name").unwrap_or_else(|| id.name.clone());
    if name != id.name {
        return Err(NopmError::with_path(
            "NP0011",
            format!(
                "npm returned package '{name}' while resolving '{}'.",
                id.name
            ),
            exact_name,
        ));
    }
    let returned_version = string_field(&metadata, "version");
    if returned_version.as_deref() != Some(&version) {
        return Err(NopmError::with_path(
            "NP0011",
            format!(
                "npm returned version '{}' while resolving '{exact_name}'.",
                returned_version.unwrap_or_else(|| "<missing>".to_owned())
            ),
            exact_name,
        ));
    }

    let mut dependencies = Vec::new();
    if let Some(map) = metadata.get("dependencies").and_then(Value::as_object) {
        for (dependency, spec) in map {
            if let Some(spec) = spec.as_str() {
                dependencies.push((dependency.clone(), spec.to_owned()));
            }
        }
    }

    let dist = metadata.get("dist").and_then(Value::as_object);
    let tarball = dist
        .and_then(|dist| dist.get("tarball"))
        .and_then(Value::as_str)
        .map(str::to_owned);
    let integrity = dist
        .and_then(|dist| dist.get("integrity"))
        .and_then(Value::as_str)
        .filter(|integrity| !integrity.is_empty())
        .map(str::to_owned);
    let (Some(tarball), Some(integrity)) = (tarball, integrity) else {
        return Err(NopmError::with_path(
            "NP0011",
            format!("npm metadata for '{exact_name}' is missing a tarball or integrity hash."),
            exact_name,
        ));
    };
    let registry = url::Url::parse(&tarball)
        .ok()
        .map(|url| url.origin().ascii_serialization());

    let exact_id = parse_package_id(&format!("npm:{name}@{version}"))?;
    let descriptor = Descriptor {
        id: exact_id.clone(),
        dependencies,
        targets: Vec::new(),
        capabilities: Vec::new(),
        resolved: Some(tarball),
        registry,
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

fn run_npm(runner: &dyn CommandRunner, args: &[&str], action: &str) -> Result<Value, NopmError> {
    let command = command_name("npm");
    let result = runner.run(&command, args, false);
    if !result.success() {
        let detail = result.stderr.trim();
        let message = if detail.is_empty() {
            format!("Unable to {action}.")
        } else {
            format!("Unable to {action}: {detail}")
        };
        return Err(NopmError::new("NP0011", message));
    }
    serde_json::from_str(&result.stdout).map_err(|_| {
        NopmError::new(
            "NP0011",
            format!("npm returned invalid JSON while trying to {action}."),
        )
    })
}

fn selected_version(value: &Value, requested: &str) -> Result<String, NopmError> {
    if let Some(version) = value.as_str().filter(|version| !version.is_empty()) {
        return Ok(version.to_owned());
    }
    if let Some(versions) = value.as_array()
        && let Some(last) = versions.iter().rev().find_map(Value::as_str)
    {
        return Ok(last.to_owned());
    }
    Err(NopmError::with_path(
        "NP0011",
        format!("npm did not return a concrete version for '{requested}'."),
        requested,
    ))
}

fn select_metadata(
    value: &Value,
    version: &str,
    requested: &str,
) -> Result<serde_json::Map<String, Value>, NopmError> {
    if let Some(record) = value.as_object() {
        return Ok(record.clone());
    }
    if let Some(entries) = value.as_array() {
        for entry in entries {
            if let Some(record) = entry.as_object()
                && record.get("version").and_then(Value::as_str) == Some(version)
            {
                return Ok(record.clone());
            }
        }
    }
    Err(NopmError::with_path(
        "NP0011",
        format!("npm did not return metadata for '{requested}'."),
        requested,
    ))
}

fn string_field(value: &serde_json::Map<String, Value>, key: &str) -> Option<String> {
    value
        .get(key)
        .and_then(Value::as_str)
        .filter(|entry| !entry.is_empty())
        .map(str::to_owned)
}
