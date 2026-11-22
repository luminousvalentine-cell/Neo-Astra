use serde_json::Value;

use crate::error::NopmError;
use crate::http::{HttpClient, encode_uri_component};
use crate::json_order::OrderedJson;
use crate::package_id::{PackageId, format_package_id, parse_dependency_request, parse_package_id};
use crate::registry_signature::{RegistryPayload, verify_registry_package};
use crate::resolve::Descriptor;

use super::{AdapterResolution, insert_dependency};

pub struct RegistryOptions<'a> {
    pub registry: &'a str,
    pub token: Option<&'a str>,
    pub require_signature: bool,
    pub public_key: Option<&'a str>,
}

pub fn resolve_registry_package(
    id: &PackageId,
    alias: &str,
    client: &dyn HttpClient,
    options: RegistryOptions<'_>,
) -> Result<AdapterResolution, NopmError> {
    if id.source != crate::package_id::Source::Nova {
        return Err(NopmError::with_path(
            "NP0011",
            "Expected a Nova registry package ID.",
            format_package_id(id),
        ));
    }
    let Some(requested_version) = id.version.clone() else {
        return Err(NopmError::with_path(
            "NP0011",
            "Nova registry packages require an exact version.",
            format_package_id(id),
        ));
    };

    let registry = options.registry.trim_end_matches('/');
    let url = format!(
        "{registry}/v1/packages/{}/{}",
        encode_uri_component(&id.name),
        encode_uri_component(&requested_version)
    );
    let mut headers: Vec<(&str, &str)> = vec![("Accept", "application/json")];
    let authorization;
    if let Some(token) = options.token {
        authorization = format!("Bearer {token}");
        headers.push(("Authorization", &authorization));
    }
    let response = client.get(&url, &headers).map_err(|message| {
        NopmError::with_path(
            "NP0011",
            format!("Unable to contact registry: {message}"),
            url.clone(),
        )
    })?;
    if !response.ok() {
        return Err(NopmError::with_path(
            "NP0011",
            format!(
                "Unable to resolve '{}': HTTP {}.",
                format_package_id(id),
                response.status
            ),
            url.clone(),
        ));
    }
    let value: Value = serde_json::from_slice(&response.body).map_err(|_| {
        NopmError::with_path("NP0011", "Registry returned invalid JSON.", url.clone())
    })?;
    let Some(value) = value.as_object() else {
        return Err(NopmError::with_path(
            "NP0011",
            "Registry returned an invalid package descriptor.",
            url.clone(),
        ));
    };

    let version = string_field(value, "version").unwrap_or_else(|| requested_version.clone());
    if version != requested_version {
        return Err(NopmError::with_path(
            "NP0011",
            format!(
                "Registry returned version '{version}' for '{}'.",
                format_package_id(id)
            ),
            url.clone(),
        ));
    }
    let Some(integrity) = string_field(value, "integrity") else {
        return Err(NopmError::with_path(
            "NP0011",
            format!(
                "Registry descriptor for '{}' is missing integrity.",
                format_package_id(id)
            ),
            url.clone(),
        ));
    };

    let mut dependencies = Vec::new();
    let ordered = OrderedJson::parse(&response.body).ok();
    if let Some(OrderedJson::Object(entries)) =
        ordered.as_ref().and_then(|value| value.get("dependencies"))
    {
        for (dependency, spec) in entries {
            if let OrderedJson::String(spec) = spec {
                insert_dependency(&mut dependencies, dependency.clone(), spec.clone());
            }
        }
    }

    let exact_id = parse_package_id(&format!("nova:{}@{version}", id.name))?;
    let targets = string_array(value.get("targets"));
    let capabilities = string_array(value.get("capabilities"));
    let resolved = string_field(value, "resolved").unwrap_or_else(|| url.clone());
    let lifecycle_scripts = value.get("lifecycleScripts").and_then(Value::as_bool) == Some(true);
    let signature = string_field(value, "signature");
    let public_key = options
        .public_key
        .map(str::to_owned)
        .or_else(|| string_field(value, "publicKey"));

    if options.require_signature && (signature.is_none() || public_key.is_none()) {
        return Err(NopmError::with_path(
            "NP0011",
            format!(
                "Registry descriptor for '{}' is not signed.",
                format_package_id(id)
            ),
            url.clone(),
        ));
    }
    if let (Some(signature), Some(public_key)) = (signature, public_key) {
        let payload = RegistryPayload {
            name: &id.name,
            version: &version,
            dependencies: &dependencies,
            targets: &targets,
            capabilities: &capabilities,
            integrity: &integrity,
            resolved: &resolved,
            lifecycle_scripts,
        };
        if !verify_registry_package(&payload, &signature, &public_key) {
            return Err(NopmError::with_path(
                "NP0011",
                format!(
                    "Registry signature verification failed for '{}'.",
                    format_package_id(id)
                ),
                url.clone(),
            ));
        }
    }

    let descriptor = Descriptor {
        id: exact_id.clone(),
        dependencies,
        targets,
        capabilities,
        resolved: Some(resolved),
        registry: Some(options.registry.to_owned()),
        integrity: Some(integrity),
        commit: None,
        lifecycle_scripts,
    };
    let request = parse_dependency_request(alias, &format_package_id(&exact_id))?;
    Ok(AdapterResolution {
        descriptor,
        request,
    })
}

fn string_field(value: &serde_json::Map<String, Value>, key: &str) -> Option<String> {
    value
        .get(key)
        .and_then(Value::as_str)
        .filter(|entry| !entry.is_empty())
        .map(str::to_owned)
}

fn string_array(value: Option<&Value>) -> Vec<String> {
    value
        .and_then(Value::as_array)
        .map(|entries| {
            entries
                .iter()
                .filter_map(Value::as_str)
                .map(str::to_owned)
                .collect()
        })
        .unwrap_or_default()
}
