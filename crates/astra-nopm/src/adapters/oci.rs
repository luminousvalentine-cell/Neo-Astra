use serde_json::Value;

use crate::error::NopmError;
use crate::http::{HttpClient, encode_uri_component};
use crate::package_id::{PackageId, format_package_id, parse_dependency_request, parse_package_id};
use crate::resolve::Descriptor;

use super::AdapterResolution;

pub fn resolve_oci_package(
    id: &PackageId,
    alias: &str,
    client: &dyn HttpClient,
) -> Result<AdapterResolution, NopmError> {
    if id.source != crate::package_id::Source::Oci || id.reference.is_none() {
        return Err(NopmError::with_path(
            "NP0011",
            "Expected an OCI package ID with a reference.",
            format_package_id(id),
        ));
    }
    let reference = id.reference.as_deref().unwrap_or("");
    let Some(slash) = id.name.find('/') else {
        return Err(NopmError::with_path(
            "NP0011",
            "OCI package names require registry/repository.",
            id.name.clone(),
        ));
    };
    if slash == 0 || slash == id.name.len() - 1 {
        return Err(NopmError::with_path(
            "NP0011",
            "OCI package names require registry/repository.",
            id.name.clone(),
        ));
    }
    let registry = &id.name[..slash];
    let repository = &id.name[slash + 1..];
    let scheme = if registry.starts_with("localhost") || registry.contains(':') {
        "http"
    } else {
        "https"
    };
    let base = format!("{scheme}://{registry}");
    let manifest_url = format!(
        "{base}/v2/{repository}/manifests/{}",
        encode_uri_component(reference)
    );

    let response = client
        .get(
            &manifest_url,
            &[(
                "Accept",
                "application/vnd.oci.image.manifest.v1+json, application/vnd.docker.distribution.manifest.v2+json",
            )],
        )
        .map_err(|message| {
            NopmError::with_path(
                "NP0011",
                format!(
                    "Unable to read OCI manifest for '{}': {message}",
                    format_package_id(id)
                ),
                manifest_url.clone(),
            )
        })?;
    if !response.ok() {
        return Err(NopmError::with_path(
            "NP0011",
            format!("Unable to read OCI manifest: HTTP {}.", response.status),
            manifest_url.clone(),
        ));
    }
    let value: Value = serde_json::from_slice(&response.body).map_err(|_| {
        NopmError::with_path(
            "NP0011",
            "OCI registry returned invalid manifest JSON.",
            manifest_url.clone(),
        )
    })?;
    let Some(layers) = value.get("layers").and_then(Value::as_array) else {
        return Err(NopmError::with_path(
            "NP0011",
            "OCI manifest has no layers.",
            manifest_url.clone(),
        ));
    };

    let layer = layers.iter().find(|entry| {
        let Some(entry) = entry.as_object() else {
            return false;
        };
        let media_type = entry.get("mediaType").and_then(Value::as_str).unwrap_or("");
        media_type.contains("tar") || media_type.contains("neoastra")
    });
    let Some(layer) = layer.and_then(Value::as_object) else {
        return Err(NopmError::with_path(
            "NP0011",
            "OCI package has no distributable tar layer.",
            manifest_url.clone(),
        ));
    };

    let digest = layer
        .get("digest")
        .and_then(Value::as_str)
        .filter(|digest| valid_digest(digest));
    let Some(digest) = digest else {
        return Err(NopmError::with_path(
            "NP0011",
            "OCI layer has no valid content digest.",
            manifest_url.clone(),
        ));
    };
    let integrity = digest.to_lowercase();
    let resolved = format!("{base}/v2/{repository}/blobs/{digest}");
    let exact_id = parse_package_id(&format!("oci:{}@{reference}", id.name))?;
    let descriptor = Descriptor {
        id: exact_id.clone(),
        dependencies: Vec::new(),
        targets: Vec::new(),
        capabilities: Vec::new(),
        resolved: Some(resolved),
        registry: Some(base),
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

fn valid_digest(digest: &str) -> bool {
    let Some((algorithm, value)) = digest.split_once(':') else {
        return false;
    };
    matches!(algorithm.to_ascii_lowercase().as_str(), "sha256" | "sha512")
        && !value.is_empty()
        && value.chars().all(|character| character.is_ascii_hexdigit())
}
