use std::collections::BTreeMap;
use std::fs;
use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

use serde::Serialize;
use serde_json::Value;

use crate::error::NopmError;
use crate::package_id::{format_package_id, locale_cmp, parse_package_id};
use crate::resolve::{LockPackage, Resolution};

pub const LOCKFILE_NAME: &str = "core.lock.json";
pub const GENERATED_BY: &str = concat!("nopm/", env!("CARGO_PKG_VERSION"));
pub const COMPILER_VERSION: &str = env!("CARGO_PKG_VERSION");
pub const COMPILER_API_VERSION: &str = "1.0";

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LockTarget {
    pub backend: String,
    pub mode: String,
    pub triple: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ToolchainRecord {
    pub rung: String,
    pub id: String,
    pub family: String,
    pub path: String,
    pub version: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CompilerRecord {
    pub version: String,
    pub api_version: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SemanticsRecord {
    pub integers: String,
    pub strings: String,
}

#[derive(Clone, Debug)]
pub struct Lockfile {
    pub generated_by: String,
    pub manifest_hash: String,
    pub compiler: CompilerRecord,
    pub semantics: SemanticsRecord,
    pub targets: BTreeMap<String, LockTarget>,
    pub packages: Vec<LockPackage>,
    pub ffi: Value,
    pub toolchain: Option<ToolchainRecord>,
}

fn lockfile_error(message: impl Into<String>) -> NopmError {
    NopmError::new("NP0008", message)
}

pub fn create_lockfile(
    resolution: &Resolution,
    manifest_hash: &str,
    ffi: Value,
    toolchain: Option<ToolchainRecord>,
) -> Result<Lockfile, NopmError> {
    if GENERATED_BY.trim().is_empty() {
        return Err(lockfile_error("Lockfile generatedBy must not be empty."));
    }
    if manifest_hash.trim().is_empty() {
        return Err(lockfile_error("Lockfile manifestHash must not be empty."));
    }
    if COMPILER_VERSION.trim().is_empty() || COMPILER_API_VERSION.trim().is_empty() {
        return Err(lockfile_error(
            "Lockfile compiler version and apiVersion must not be empty.",
        ));
    }

    let mut targets: BTreeMap<String, LockTarget> = BTreeMap::new();
    let mut sorted_targets: Vec<&crate::project::TargetSpec> = resolution.targets.iter().collect();
    sorted_targets.sort_by(|left, right| locale_cmp(&left.name, &right.name));
    for target in sorted_targets {
        if targets.contains_key(&target.name) {
            return Err(NopmError::with_path(
                "NP0006",
                format!("Duplicate target '{}' in resolution.", target.name),
                target.name.clone(),
            ));
        }
        targets.insert(
            target.name.clone(),
            LockTarget {
                backend: target.backend.clone(),
                mode: target.mode.clone(),
                triple: target.triple.clone(),
            },
        );
    }

    let mut packages: Vec<LockPackage> = resolution.packages.clone();
    packages.sort_by(|left, right| locale_cmp(&left.id, &right.id));
    for entry in &packages {
        if entry.source != "path"
            && entry
                .integrity
                .as_ref()
                .map(|integrity| integrity.is_empty())
                .unwrap_or(true)
        {
            return Err(NopmError::with_path(
                "NP0008",
                format!("Remote package '{}' requires an integrity hash.", entry.id),
                entry.id.clone(),
            ));
        }
        if entry.source == "git"
            && entry
                .commit
                .as_ref()
                .map(|commit| commit.is_empty())
                .unwrap_or(true)
        {
            return Err(NopmError::with_path(
                "NP0008",
                format!("Git package '{}' requires a pinned commit.", entry.id),
                entry.id.clone(),
            ));
        }
    }

    Ok(Lockfile {
        generated_by: GENERATED_BY.to_owned(),
        manifest_hash: manifest_hash.to_owned(),
        compiler: CompilerRecord {
            version: COMPILER_VERSION.to_owned(),
            api_version: COMPILER_API_VERSION.to_owned(),
        },
        semantics: SemanticsRecord {
            integers: "wrapping".to_owned(),
            strings: "utf8".to_owned(),
        },
        targets,
        packages,
        ffi,
        toolchain,
    })
}

#[derive(Serialize)]
struct TargetDocument<'a> {
    backend: &'a str,
    #[serde(skip_serializing_if = "Option::is_none")]
    triple: Option<&'a str>,
    mode: &'a str,
}

#[derive(Serialize)]
struct PackageDocument<'a> {
    id: &'a str,
    name: &'a str,
    source: &'a str,
    version: Option<&'a str>,
    resolved: Option<&'a str>,
    registry: Option<&'a str>,
    integrity: Option<&'a str>,
    targets: &'a [String],
    capabilities: &'a [String],
    #[serde(rename = "lifecycleScripts")]
    lifecycle_scripts: bool,
    dependencies: &'a [String],
    #[serde(skip_serializing_if = "Option::is_none")]
    commit: Option<&'a str>,
}

#[derive(Serialize)]
struct ToolchainDocument<'a> {
    rung: &'a str,
    id: &'a str,
    family: &'a str,
    path: &'a str,
    version: Option<&'a str>,
}

#[derive(Serialize)]
struct LockfileDocument<'a> {
    #[serde(rename = "lockfileVersion")]
    lockfile_version: u8,
    #[serde(rename = "generatedBy")]
    generated_by: &'a str,
    #[serde(rename = "manifestHash")]
    manifest_hash: &'a str,
    compiler: CompilerDocument<'a>,
    semantics: SemanticsDocument<'a>,
    targets: BTreeMap<String, TargetDocument<'a>>,
    packages: Vec<PackageDocument<'a>>,
    ffi: &'a Value,
    toolchain: Option<ToolchainDocument<'a>>,
}

#[derive(Serialize)]
struct CompilerDocument<'a> {
    version: &'a str,
    #[serde(rename = "apiVersion")]
    api_version: &'a str,
}

#[derive(Serialize)]
struct SemanticsDocument<'a> {
    integers: &'a str,
    strings: &'a str,
}

pub fn format_lockfile(lockfile: &Lockfile) -> String {
    let targets = lockfile
        .targets
        .iter()
        .map(|(name, target)| {
            (
                name.clone(),
                TargetDocument {
                    backend: &target.backend,
                    triple: target.triple.as_deref(),
                    mode: &target.mode,
                },
            )
        })
        .collect();

    let packages = lockfile
        .packages
        .iter()
        .map(|entry| PackageDocument {
            id: &entry.id,
            name: &entry.name,
            source: &entry.source,
            version: entry.version.as_deref(),
            resolved: entry.resolved.as_deref(),
            registry: entry.registry.as_deref(),
            integrity: entry.integrity.as_deref(),
            targets: &entry.targets,
            capabilities: &entry.capabilities,
            lifecycle_scripts: entry.lifecycle_scripts,
            dependencies: &entry.dependencies,
            commit: entry.commit.as_deref(),
        })
        .collect();

    let document = LockfileDocument {
        lockfile_version: 1,
        generated_by: &lockfile.generated_by,
        manifest_hash: &lockfile.manifest_hash,
        compiler: CompilerDocument {
            version: &lockfile.compiler.version,
            api_version: &lockfile.compiler.api_version,
        },
        semantics: SemanticsDocument {
            integers: &lockfile.semantics.integers,
            strings: &lockfile.semantics.strings,
        },
        targets,
        packages,
        ffi: &lockfile.ffi,
        toolchain: lockfile
            .toolchain
            .as_ref()
            .map(|toolchain| ToolchainDocument {
                rung: &toolchain.rung,
                id: &toolchain.id,
                family: &toolchain.family,
                path: &toolchain.path,
                version: toolchain.version.as_deref(),
            }),
    };

    format!(
        "{}\n",
        serde_json::to_string_pretty(&document).unwrap_or_else(|_| "{}".to_owned())
    )
}

fn require_record<'a>(
    value: &'a serde_json::Map<String, Value>,
    key: &str,
    label: &str,
) -> Result<&'a serde_json::Map<String, Value>, NopmError> {
    value
        .get(key)
        .and_then(Value::as_object)
        .ok_or_else(|| lockfile_error(format!("{label}.{key} must be an object.")))
}

fn require_array<'a>(
    value: &'a serde_json::Map<String, Value>,
    key: &str,
    label: &str,
) -> Result<&'a Vec<Value>, NopmError> {
    value
        .get(key)
        .and_then(Value::as_array)
        .ok_or_else(|| lockfile_error(format!("{label}.{key} must be an array.")))
}

fn require_string(
    value: &serde_json::Map<String, Value>,
    key: &str,
    label: &str,
) -> Result<String, NopmError> {
    value
        .get(key)
        .and_then(Value::as_str)
        .filter(|text| !text.is_empty())
        .map(|text| text.to_owned())
        .ok_or_else(|| lockfile_error(format!("{label}.{key} must be a non-empty string.")))
}

fn nullable_string(
    value: &serde_json::Map<String, Value>,
    key: &str,
    label: &str,
) -> Result<Option<String>, NopmError> {
    match value.get(key) {
        Some(Value::Null) => Ok(None),
        Some(Value::String(text)) => Ok(Some(text.clone())),
        _ => Err(lockfile_error(format!(
            "{label}.{key} must be a string or null."
        ))),
    }
}

fn optional_nullable_string(
    value: &serde_json::Map<String, Value>,
    key: &str,
    label: &str,
) -> Result<Option<String>, NopmError> {
    if value.contains_key(key) {
        nullable_string(value, key, label)
    } else {
        Ok(None)
    }
}

fn require_boolean(
    value: &serde_json::Map<String, Value>,
    key: &str,
    label: &str,
) -> Result<bool, NopmError> {
    value
        .get(key)
        .and_then(Value::as_bool)
        .ok_or_else(|| lockfile_error(format!("{label}.{key} must be a boolean.")))
}

fn string_array(
    value: &serde_json::Map<String, Value>,
    key: &str,
    label: &str,
) -> Result<Vec<String>, NopmError> {
    let items = value
        .get(key)
        .and_then(Value::as_array)
        .ok_or_else(|| lockfile_error(format!("{label}.{key} must be an array of strings.")))?;
    let mut result = Vec::new();
    for item in items {
        let Some(text) = item.as_str() else {
            return Err(lockfile_error(format!(
                "{label}.{key} must be an array of strings."
            )));
        };
        result.push(text.to_owned());
    }
    Ok(result)
}

pub fn parse_lockfile(source: &str) -> Result<Lockfile, NopmError> {
    let value: Value =
        serde_json::from_str(source).map_err(|_| lockfile_error("Invalid lockfile JSON."))?;
    validate_lockfile(&value)
}

pub fn validate_lockfile(value: &Value) -> Result<Lockfile, NopmError> {
    let Some(root) = value.as_object() else {
        return Err(lockfile_error(
            "Lockfile must be an object with lockfileVersion 1.",
        ));
    };
    if root.get("lockfileVersion").and_then(Value::as_u64) != Some(1) {
        return Err(lockfile_error(
            "Lockfile must be an object with lockfileVersion 1.",
        ));
    }

    let generated_by = require_string(root, "generatedBy", "lockfile")?;
    let manifest_hash = require_string(root, "manifestHash", "lockfile")?;
    let compiler = require_record(root, "compiler", "lockfile")?;
    let semantics = require_record(root, "semantics", "lockfile")?;
    let targets_raw = require_record(root, "targets", "lockfile")?;
    let packages_raw = require_array(root, "packages", "lockfile")?;
    let ffi = require_record(root, "ffi", "lockfile")?;

    let toolchain = match root.get("toolchain") {
        None | Some(Value::Null) => None,
        Some(Value::Object(raw)) => {
            let rung = require_string(raw, "rung", "toolchain")?;
            if !matches!(rung.as_str(), "env" | "cache" | "path") {
                return Err(NopmError::with_path(
                    "NP0008",
                    format!("Toolchain rung '{rung}' is invalid."),
                    "toolchain",
                ));
            }
            Some(ToolchainRecord {
                rung,
                id: require_string(raw, "id", "toolchain")?,
                family: require_string(raw, "family", "toolchain")?,
                path: require_string(raw, "path", "toolchain")?,
                version: optional_nullable_string(raw, "version", "toolchain")?,
            })
        }
        Some(_) => {
            return Err(lockfile_error(
                "Lockfile 'toolchain' must be an object or null.",
            ));
        }
    };

    let mut targets: BTreeMap<String, LockTarget> = BTreeMap::new();
    let mut target_names: Vec<&String> = targets_raw.keys().collect();
    target_names.sort();
    for name in target_names {
        let Some(raw) = targets_raw[name].as_object() else {
            return Err(lockfile_error(format!(
                "Target '{name}' must be an object."
            )));
        };
        let backend = require_string(raw, "backend", &format!("target '{name}'"))?;
        let mode = require_string(raw, "mode", &format!("target '{name}'"))?;
        if !crate::project::is_mode(&mode) {
            return Err(NopmError::with_path(
                "NP0008",
                format!("Target '{name}' has invalid mode '{mode}'."),
                name.clone(),
            ));
        }
        let triple = optional_nullable_string(raw, "triple", &format!("target '{name}'"))?;
        targets.insert(
            name.clone(),
            LockTarget {
                backend,
                mode,
                triple,
            },
        );
    }

    let mut packages = Vec::new();
    for (index, entry) in packages_raw.iter().enumerate() {
        packages.push(validate_lock_package(entry, index)?);
    }

    let mut seen = std::collections::BTreeSet::new();
    let mut previous = String::new();
    for entry in &packages {
        if !seen.insert(entry.id.clone()) {
            return Err(NopmError::with_path(
                "NP0008",
                format!("Duplicate lockfile package '{}'.", entry.id),
                entry.id.clone(),
            ));
        }
        if locale_cmp(&entry.id, &previous) == std::cmp::Ordering::Less {
            return Err(NopmError::with_path(
                "NP0008",
                "Lockfile packages must be sorted by canonical package ID.",
                entry.id.clone(),
            ));
        }
        previous = entry.id.clone();
    }

    Ok(Lockfile {
        generated_by,
        manifest_hash,
        compiler: CompilerRecord {
            version: require_string(compiler, "version", "lockfile compiler")?,
            api_version: require_string(compiler, "apiVersion", "lockfile compiler")?,
        },
        semantics: SemanticsRecord {
            integers: require_string(semantics, "integers", "lockfile semantics")?,
            strings: require_string(semantics, "strings", "lockfile semantics")?,
        },
        targets,
        packages,
        ffi: Value::Object(ffi.clone()),
        toolchain,
    })
}

fn validate_lock_package(value: &Value, index: usize) -> Result<LockPackage, NopmError> {
    let label = format!("package[{index}]");
    let Some(raw) = value.as_object() else {
        return Err(lockfile_error(format!("{label} must be an object.")));
    };

    let raw_id = require_string(raw, "id", &label)?;
    let id = parse_package_id(&raw_id).map_err(|_| {
        NopmError::with_path(
            "NP0008",
            format!("{label} has an invalid package ID '{raw_id}'."),
            raw_id.clone(),
        )
    })?;
    if format_package_id(&id) != raw_id {
        return Err(NopmError::with_path(
            "NP0008",
            format!("{label} package ID is not canonical."),
            raw_id.clone(),
        ));
    }

    let source = require_string(raw, "source", &label)?;
    if source != id.source.as_str() {
        return Err(NopmError::with_path(
            "NP0008",
            format!("{label} source does not match its package ID."),
            raw_id.clone(),
        ));
    }

    let name = require_string(raw, "name", &label)?;
    if name != id.name {
        return Err(NopmError::with_path(
            "NP0008",
            format!("{label} name does not match its package ID."),
            raw_id.clone(),
        ));
    }
    let version = nullable_string(raw, "version", &label)?;
    let resolved = nullable_string(raw, "resolved", &label)?;
    let registry = nullable_string(raw, "registry", &label)?;
    let integrity = nullable_string(raw, "integrity", &label)?;
    let targets = string_array(raw, "targets", &label)?;
    let capabilities = string_array(raw, "capabilities", &label)?;
    let lifecycle_scripts = require_boolean(raw, "lifecycleScripts", &label)?;
    let dependencies = string_array(raw, "dependencies", &label)?;
    let commit = optional_nullable_string(raw, "commit", &label)?;

    if version != id.version {
        return Err(NopmError::with_path(
            "NP0008",
            format!("{label} version does not match its package ID."),
            raw_id.clone(),
        ));
    }
    if id.source.as_str() != "path"
        && integrity
            .as_ref()
            .map(|value| value.is_empty())
            .unwrap_or(true)
    {
        return Err(NopmError::with_path(
            "NP0008",
            format!("{label} remote package requires integrity."),
            raw_id.clone(),
        ));
    }
    if id.source.as_str() == "git"
        && commit
            .as_ref()
            .map(|value| value.is_empty())
            .unwrap_or(true)
    {
        return Err(NopmError::with_path(
            "NP0008",
            format!("{label} git package requires a pinned commit."),
            raw_id.clone(),
        ));
    }

    let mut canonical_dependencies = dependencies.clone();
    canonical_dependencies.sort();
    canonical_dependencies.dedup();
    if canonical_dependencies != dependencies {
        return Err(NopmError::with_path(
            "NP0008",
            format!("{label} dependencies must be unique and sorted by canonical ID."),
            raw_id.clone(),
        ));
    }

    for dependency in &dependencies {
        match parse_package_id(dependency) {
            Ok(dependency_id) if format_package_id(&dependency_id) == *dependency => {}
            _ => {
                return Err(NopmError::with_path(
                    "NP0008",
                    format!("{label} has an invalid dependency ID."),
                    dependency.clone(),
                ));
            }
        }
    }

    Ok(LockPackage {
        id: raw_id,
        name,
        source,
        version,
        resolved,
        registry,
        integrity,
        targets,
        capabilities,
        lifecycle_scripts,
        dependencies,
        commit,
    })
}

pub fn read_lockfile_file(path: &Path) -> Result<Lockfile, NopmError> {
    let display = path.to_string_lossy().into_owned();
    let source = match fs::read_to_string(path) {
        Ok(source) => source,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Err(NopmError::with_path(
                "NP0009",
                format!("Lockfile '{display}' does not exist."),
                display,
            ));
        }
        Err(_) => {
            return Err(NopmError::with_path(
                "NP0008",
                format!("Unable to read lockfile '{display}'."),
                display,
            ));
        }
    };
    parse_lockfile(&source)
}

pub fn write_lockfile_file(path: &Path, lockfile: &Lockfile) -> Result<(), NopmError> {
    let display = path.to_string_lossy().into_owned();
    let source = format_lockfile(lockfile);
    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_nanos())
        .unwrap_or(0);
    let temporary = path.with_file_name(format!(
        "{}.tmp-{}-{}",
        path.file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_else(|| LOCKFILE_NAME.to_owned()),
        std::process::id(),
        unique
    ));
    let result = fs::write(&temporary, source).and_then(|_| fs::rename(&temporary, path));
    if result.is_err() {
        let _ = fs::remove_file(&temporary);
        return Err(NopmError::with_path(
            "NP0010",
            format!("Unable to write lockfile '{display}'."),
            display,
        ));
    }
    Ok(())
}

pub fn is_lockfile_current(lockfile: &Lockfile, manifest_hash: &str) -> bool {
    lockfile.manifest_hash == manifest_hash
}

pub fn assert_lockfile_current<'a>(
    lockfile: &'a Lockfile,
    manifest_hash: &str,
) -> Result<&'a Lockfile, NopmError> {
    if lockfile.manifest_hash != manifest_hash {
        return Err(NopmError::with_path(
            "NP0009",
            format!(
                "Lockfile is stale. Expected {manifest_hash}, found {}.",
                lockfile.manifest_hash
            ),
            LOCKFILE_NAME,
        ));
    }
    Ok(lockfile)
}
