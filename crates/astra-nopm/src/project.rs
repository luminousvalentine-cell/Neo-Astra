use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Component, Path, PathBuf};

use serde_json::Value;

use crate::error::NopmError;
use crate::manifest::{
    ManifestFile, find_build_manifest, find_core_manifest, parse_manifest_source,
    validate_manifest_keys,
};
use crate::package_id::{DependencyRequest, locale_cmp, parse_dependency_request};
use crate::sha256::sha256_hex;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TargetSpec {
    pub name: String,
    pub backend: String,
    pub mode: String,
    pub triple: Option<String>,
}

#[derive(Clone, Debug)]
pub struct ProjectDependency {
    pub alias: String,
    pub request: DependencyRequest,
    pub targets: Vec<String>,
}

#[derive(Clone, Debug)]
pub struct ProjectManifest {
    pub directory: PathBuf,
    pub file: ManifestFile,
    pub source: String,
    pub manifest_hash: String,
    pub package_name: Option<String>,
    pub package_version: Option<String>,
    pub dependencies: Vec<ProjectDependency>,
    pub targets: Vec<TargetSpec>,
    pub registry: Option<String>,
    pub default_source: String,
    pub package_sources: BTreeMap<String, bool>,
    pub capabilities: Vec<String>,
    pub ffi: Value,
    pub build_data: Value,
}

fn is_record(value: &Value) -> bool {
    value.is_object()
}

fn record_value<'a>(data: &'a Value, key: &str) -> &'a serde_json::Map<String, Value> {
    static EMPTY: std::sync::OnceLock<serde_json::Map<String, Value>> = std::sync::OnceLock::new();
    match data.get(key).filter(|value| is_record(value)) {
        Some(Value::Object(map)) => map,
        _ => EMPTY.get_or_init(serde_json::Map::new),
    }
}

fn string_value(map: &serde_json::Map<String, Value>, key: &str) -> Option<String> {
    map.get(key)
        .and_then(Value::as_str)
        .map(|value| value.to_owned())
}

pub fn absolute_path(path: &Path) -> PathBuf {
    let joined = if path.is_absolute() {
        path.to_path_buf()
    } else {
        std::env::current_dir()
            .unwrap_or_else(|_| PathBuf::from("."))
            .join(path)
    };

    let mut normalized = PathBuf::new();
    for component in joined.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                let popped = normalized.pop();
                if !popped {
                    normalized.push(component.as_os_str());
                }
            }
            other => normalized.push(other.as_os_str()),
        }
    }
    normalized
}

pub fn hash_manifest_source(source: &str) -> String {
    let normalized = source.trim_start_matches('\u{FEFF}').replace("\r\n", "\n");
    format!("sha256:{}", sha256_hex(normalized.as_bytes()))
}

fn read_core_with_source(file: &ManifestFile) -> Result<(String, Value), NopmError> {
    let source = fs::read_to_string(&file.path).map_err(|error| {
        NopmError::with_path(
            "NS9002",
            format!("Unable to read manifest: {error}"),
            file.display_path(),
        )
    })?;
    let data = parse_manifest_source(&source, file)?;
    validate_manifest_keys(file, &data)?;
    Ok((source, data))
}

fn read_optional_build(directory: &Path) -> Result<(Option<ManifestFile>, Value), NopmError> {
    let Some(file) = find_build_manifest(directory)? else {
        return Ok((None, Value::Object(serde_json::Map::new())));
    };
    let source = fs::read_to_string(&file.path).map_err(|error| {
        NopmError::with_path(
            "NS9002",
            format!("Unable to read manifest: {error}"),
            file.display_path(),
        )
    })?;
    let data = parse_manifest_source(&source, &file)?;
    validate_manifest_keys(&file, &data)?;
    Ok((Some(file), data))
}

pub fn read_project(directory: &Path, require_targets: bool) -> Result<ProjectManifest, NopmError> {
    let absolute = absolute_path(directory);
    let file = find_core_manifest(&absolute)?;
    let (source, core_data) = read_core_with_source(&file)?;
    let (_, build_data) = read_optional_build(&absolute)?;

    let mut data = core_data;
    if let (Some(target), Value::Object(source_map)) = (data.as_object_mut(), build_data.clone()) {
        for (key, value) in source_map {
            target.insert(key, value);
        }
    }

    let package = record_value(&data, "package");
    let registry_record = record_value(&data, "registry");
    let capabilities_record = record_value(&data, "capabilities");

    let dependencies = read_dependencies(&data, registry_record)?;
    let targets = read_targets(&data, require_targets)?;

    Ok(ProjectManifest {
        directory: absolute,
        file,
        manifest_hash: hash_manifest_source(&source),
        package_name: string_value(package, "name"),
        package_version: string_value(package, "version"),
        dependencies,
        targets,
        registry: string_value(registry_record, "default"),
        default_source: string_value(registry_record, "defaultSource")
            .unwrap_or_else(|| "nova".to_owned()),
        package_sources: read_package_sources(&data),
        capabilities: collect_capabilities(capabilities_record),
        ffi: record_value(&data, "ffi").clone().into(),
        build_data,
        source,
    })
}

fn read_dependencies(
    data: &Value,
    registry: &serde_json::Map<String, Value>,
) -> Result<Vec<ProjectDependency>, NopmError> {
    let raw = record_value(data, "dependencies");
    let default_source =
        string_value(registry, "defaultSource").unwrap_or_else(|| "nova".to_owned());
    let mut aliases: Vec<&String> = raw.keys().collect();
    aliases.sort_by(|left, right| locale_cmp(left, right));

    let mut dependencies = Vec::new();
    for alias in aliases {
        let declaration = &raw[alias];
        let request = dependency_request(alias, declaration, &default_source)?;
        let targets = if is_record(declaration) {
            match declaration.get("targets") {
                Some(Value::Array(items)) => string_array(
                    items,
                    &format!("dependency '{alias}' targets"),
                    &format!("dependency '{alias}' targets"),
                )?,
                Some(_) => {
                    return Err(NopmError::with_path(
                        "NP0002",
                        format!("dependency '{alias}' targets must be an array of strings."),
                        format!("dependency '{alias}' targets"),
                    ));
                }
                None => Vec::new(),
            }
        } else {
            Vec::new()
        };
        dependencies.push(ProjectDependency {
            alias: alias.clone(),
            request,
            targets,
        });
    }
    Ok(dependencies)
}

fn string_array(items: &[Value], label: &str, path: &str) -> Result<Vec<String>, NopmError> {
    let mut values = Vec::new();
    for item in items {
        match item.as_str() {
            Some(value) => values.push(value.to_owned()),
            None => {
                return Err(NopmError::with_path(
                    "NP0002",
                    format!("{label} must be an array of strings."),
                    path,
                ));
            }
        }
    }
    Ok(values)
}

fn dependency_request(
    alias: &str,
    declaration: &Value,
    default_source: &str,
) -> Result<DependencyRequest, NopmError> {
    if let Some(spec) = declaration.as_str() {
        return parse_dependency_request(alias, spec);
    }
    if !is_record(declaration) {
        return Err(NopmError::with_path(
            "NP0002",
            format!("Dependency '{alias}' must be a source-qualified string or an object."),
            alias,
        ));
    }
    let declaration = declaration.as_object().unwrap();
    let source = string_value(declaration, "source").unwrap_or_else(|| default_source.to_owned());
    let name = string_value(declaration, "name").unwrap_or_else(|| alias.to_owned());
    let version = string_value(declaration, "version");
    let path = string_value(declaration, "path");
    let url = string_value(declaration, "url");
    let reference =
        string_value(declaration, "ref").or_else(|| string_value(declaration, "reference"));

    let spec = match source.as_str() {
        "path" => {
            let Some(path) = path else {
                return Err(NopmError::with_path(
                    "NP0002",
                    format!("Path dependency '{alias}' requires a path."),
                    alias,
                ));
            };
            format!("path:{path}")
        }
        "git" => {
            let (Some(url), Some(reference)) = (url, reference) else {
                return Err(NopmError::with_path(
                    "NP0002",
                    format!("Git dependency '{alias}' requires url and ref."),
                    alias,
                ));
            };
            format!("git+{url}#{reference}")
        }
        "nova" | "npm" | "cargo" | "pypi" | "nuget" => match version {
            Some(version) => format!("{source}:{name}@{version}"),
            None => format!("{source}:{name}"),
        },
        "oci" => {
            let (Some(url), Some(reference)) = (url, reference) else {
                return Err(NopmError::with_path(
                    "NP0002",
                    format!("OCI dependency '{alias}' requires url and reference."),
                    alias,
                ));
            };
            format!("oci:{url}@{reference}")
        }
        other => {
            return Err(NopmError::with_path(
                "NP0002",
                format!("Dependency '{alias}' uses unsupported source '{other}'."),
                alias,
            ));
        }
    };

    parse_dependency_request(alias, &spec)
}

fn read_targets(data: &Value, require_targets: bool) -> Result<Vec<TargetSpec>, NopmError> {
    let raw = record_value(data, "targets");
    let build = record_value(data, "build");
    let default_mode = string_value(build, "defaultMode");

    let mut names: Vec<&String> = raw.keys().collect();
    names.sort_by(|left, right| locale_cmp(left, right));

    let mut targets = Vec::new();
    for name in names {
        let declaration = &raw[name];
        let Some(declaration) = declaration.as_object() else {
            return Err(NopmError::with_path(
                "NP0006",
                format!("Target '{name}' must be an object."),
                name,
            ));
        };
        let Some(backend) = string_value(declaration, "backend") else {
            return Err(NopmError::with_path(
                "NP0006",
                format!("Target '{name}' requires a backend."),
                name,
            ));
        };
        let mode = string_value(declaration, "mode")
            .or_else(|| default_mode.clone())
            .unwrap_or_else(|| default_mode_for(name, &backend).to_owned());
        if !is_mode(&mode) {
            return Err(NopmError::with_path(
                "NP0007",
                format!("Target '{name}' has invalid mode '{mode}'."),
                mode,
            ));
        }
        let triple = string_value(declaration, "triple");
        targets.push(TargetSpec {
            name: name.clone(),
            backend,
            mode,
            triple,
        });
    }

    if targets.is_empty()
        && let Some(default_target) = string_value(build, "defaultTarget")
    {
        let mode = match default_mode {
            Some(mode) if is_mode(&mode) => mode,
            _ => "web".to_owned(),
        };
        targets.push(TargetSpec {
            name: default_target,
            backend: "js".to_owned(),
            mode,
            triple: None,
        });
    }

    if targets.is_empty() && require_targets {
        return Err(NopmError::with_path(
            "NP0006",
            "No project targets are configured. Add a [targets] entry to the manifest.",
            "targets",
        ));
    }

    Ok(targets)
}

fn default_mode_for(name: &str, backend: &str) -> &'static str {
    if backend == "js" || backend == "ts" || name.contains("web") {
        "web"
    } else if backend == "c"
        || backend == "wasm32"
        || name.contains("native")
        || name.contains("wasi")
    {
        "system"
    } else {
        "core"
    }
}

pub fn is_mode(mode: &str) -> bool {
    matches!(mode, "core" | "app" | "web" | "system")
}

fn read_package_sources(data: &Value) -> BTreeMap<String, bool> {
    let raw = record_value(data, "packageSources");
    let mut sources = BTreeMap::new();
    for (key, value) in raw {
        if let Some(enabled) = value.as_bool() {
            sources.insert(key.clone(), enabled);
        } else if let Some(object) = value.as_object() {
            sources.insert(
                key.clone(),
                object.get("enabled").and_then(Value::as_bool) == Some(true),
            );
        }
    }
    sources
}

fn collect_capabilities(raw: &serde_json::Map<String, Value>) -> Vec<String> {
    let mut capabilities = BTreeSet::new();
    for value in raw.values() {
        if let Some(items) = value.as_array() {
            for entry in items {
                if let Some(capability) = entry.as_str() {
                    capabilities.insert(capability.to_owned());
                }
            }
        }
    }
    capabilities.into_iter().collect()
}

pub fn package_directory(project_root: &Path, path: &str) -> Result<PathBuf, NopmError> {
    let target = absolute_path(&project_root.join(path));
    let normalized_root = project_root.to_string_lossy().replace('\\', "/");
    let normalized_root = normalized_root.trim_end_matches('/').to_owned();
    let normalized_target = target.to_string_lossy().replace('\\', "/");
    if normalized_target != normalized_root
        && !normalized_target.starts_with(&format!("{normalized_root}/"))
    {
        return Err(NopmError::with_path(
            "NP0011",
            format!("Path package '{path}' escapes the project root."),
            path,
        ));
    }
    Ok(target)
}
