use std::env;
use std::fs;
use std::path::{Path, PathBuf};

use crate::CliError;

const CORE_JSON: &str = "core.json";
const CORE_TOML: &str = "core.toml";
const BUILD_JSON: &str = "build.json";
const BUILD_TOML: &str = "build.toml";
const DEFAULT_ENTRY: &str = "src/main.ns";

pub(crate) fn resolve_entry(command: &'static str, json: bool) -> Result<String, CliError> {
    let cwd = env::current_dir().map_err(|error| {
        CliError::input(
            "NS9008",
            format!("Unable to determine the current directory: {error}"),
            command,
            ".".to_owned(),
            json,
        )
    })?;
    let root = fs::canonicalize(&cwd).map_err(|error| {
        CliError::input(
            "NS9008",
            format!("Unable to resolve the project directory: {error}"),
            command,
            cwd.display().to_string(),
            json,
        )
    })?;

    resolve_core_manifest(&root, command, json)?;
    let build_path = resolve_build_manifest(&root, command, json)?;

    let entry = match &build_path {
        Some(path) => {
            let source = read_manifest(path, command, json)?;
            if path.extension().is_some_and(|value| value == "json") {
                parse_json_entry(&source, path, command, json)?
            } else {
                parse_toml_entry(&source, path, command, json)?
            }
        }
        None => DEFAULT_ENTRY.to_owned(),
    };

    let entry_path = Path::new(&entry);
    let error_path = build_path.as_deref().unwrap_or(&root);
    if entry_path.is_absolute() {
        return Err(manifest_error(
            "The [nova].entry path must be relative to the project root.",
            error_path,
            command,
            json,
        ));
    }
    if !entry_path
        .extension()
        .and_then(|value| value.to_str())
        .is_some_and(|value| value.eq_ignore_ascii_case("ns"))
    {
        return Err(manifest_error(
            "[nova].entry must reference a .ns file.",
            error_path,
            command,
            json,
        ));
    }

    let resolved = fs::canonicalize(root.join(entry_path)).map_err(|error| {
        CliError::input(
            "NS9008",
            format!("Unable to resolve the [nova].entry file: {error}"),
            command,
            root.join(entry_path).display().to_string(),
            json,
        )
    })?;

    if !resolved.starts_with(&root) {
        return Err(manifest_error(
            "[nova].entry must stay inside the project root.",
            error_path,
            command,
            json,
        ));
    }
    if !resolved.is_file() {
        return Err(CliError::input(
            "NS9008",
            "The [nova].entry target is not a file.",
            command,
            resolved.display().to_string(),
            json,
        ));
    }

    Ok(resolved.display().to_string())
}

fn resolve_core_manifest(
    root: &Path,
    command: &'static str,
    json: bool,
) -> Result<PathBuf, CliError> {
    let json_path = root.join(CORE_JSON);
    let toml_path = root.join(CORE_TOML);
    match (json_path.is_file(), toml_path.is_file()) {
        (true, false) => Ok(json_path),
        (false, true) => Ok(toml_path),
        (false, false) => Err(CliError::input(
            "NS9001",
            "No manifest found. Expected exactly one of core.json or core.toml.",
            command,
            root.display().to_string(),
            json,
        )),
        (true, true) => Err(CliError::input(
            "NS9001",
            "Both core.json and core.toml exist. Exactly one manifest is allowed.",
            command,
            root.display().to_string(),
            json,
        )),
    }
}

fn resolve_build_manifest(
    root: &Path,
    command: &'static str,
    json: bool,
) -> Result<Option<PathBuf>, CliError> {
    let json_path = root.join(BUILD_JSON);
    let toml_path = root.join(BUILD_TOML);
    match (json_path.is_file(), toml_path.is_file()) {
        (true, false) => Ok(Some(json_path)),
        (false, true) => Ok(Some(toml_path)),
        (false, false) => Ok(None),
        (true, true) => Err(CliError::input(
            "NS9001",
            "Both build.json and build.toml exist. Exactly one build manifest is allowed.",
            command,
            root.display().to_string(),
            json,
        )),
    }
}

fn read_manifest(
    manifest_path: &Path,
    command: &'static str,
    json: bool,
) -> Result<String, CliError> {
    let source = fs::read(manifest_path).map_err(|error| {
        CliError::input(
            "NS9002",
            format!("Unable to read the build manifest: {error}"),
            command,
            manifest_path.display().to_string(),
            json,
        )
    })?;
    String::from_utf8(source).map_err(|_| {
        CliError::input(
            "NS9002",
            "The build manifest is not valid UTF-8.",
            command,
            manifest_path.display().to_string(),
            json,
        )
    })
}

fn parse_json_entry(
    source: &str,
    manifest_path: &Path,
    command: &'static str,
    json: bool,
) -> Result<String, CliError> {
    let value: serde_json::Value = serde_json::from_str(source).map_err(|error| {
        CliError::input(
            "NS9002",
            format!("Invalid JSON build manifest: {error}"),
            command,
            manifest_path.display().to_string(),
            json,
        )
    })?;

    let Some(nova) = value.get("nova") else {
        return Ok(DEFAULT_ENTRY.to_owned());
    };
    if !nova.is_object() {
        return Err(manifest_error(
            "Invalid [nova] section in build.json.",
            manifest_path,
            command,
            json,
        ));
    }
    match nova.get("entry") {
        None => Ok(DEFAULT_ENTRY.to_owned()),
        Some(serde_json::Value::String(entry)) => Ok(entry.clone()),
        Some(_) => Err(manifest_error(
            "Invalid [nova].entry in build.json: expected a string.",
            manifest_path,
            command,
            json,
        )),
    }
}

fn parse_toml_entry(
    source: &str,
    manifest_path: &Path,
    command: &'static str,
    json: bool,
) -> Result<String, CliError> {
    let value: toml::Value = toml::from_str(source).map_err(|error| {
        CliError::input(
            "NS9002",
            format!("Invalid TOML build manifest: {error}"),
            command,
            manifest_path.display().to_string(),
            json,
        )
    })?;

    let Some(nova) = value.get("nova") else {
        return Ok(DEFAULT_ENTRY.to_owned());
    };
    let Some(table) = nova.as_table() else {
        return Err(manifest_error(
            "Invalid [nova] section in build.toml.",
            manifest_path,
            command,
            json,
        ));
    };
    match table.get("entry") {
        None => Ok(DEFAULT_ENTRY.to_owned()),
        Some(toml::Value::String(entry)) => Ok(entry.clone()),
        Some(_) => Err(manifest_error(
            "Invalid [nova].entry in build.toml: expected a string.",
            manifest_path,
            command,
            json,
        )),
    }
}

fn manifest_error(
    message: impl Into<String>,
    manifest_path: &Path,
    command: &'static str,
    json: bool,
) -> CliError {
    CliError::input(
        "NS9002",
        message,
        command,
        manifest_path.display().to_string(),
        json,
    )
}
