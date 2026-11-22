use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};

use serde_json::{Map, Value};

use crate::error::NopmError;
use crate::package_id::locale_cmp;

pub const CORE_MANIFEST_NAMES: [&str; 2] = ["core.json", "core.toml"];
pub const BUILD_MANIFEST_NAMES: [&str; 2] = ["build.json", "build.toml"];

pub const CORE_KEYS: [&str; 6] = [
    "package",
    "dependencies",
    "packageSources",
    "registry",
    "ffi",
    "capabilities",
];

pub const BUILD_KEYS: [&str; 7] = [
    "build",
    "targets",
    "nova",
    "math",
    "strings",
    "memory",
    "toolchain",
];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ManifestFormat {
    Json,
    Toml,
}

impl ManifestFormat {
    fn extension(self) -> &'static str {
        match self {
            ManifestFormat::Json => "json",
            ManifestFormat::Toml => "toml",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ManifestKind {
    Core,
    Build,
}

#[derive(Clone, Debug)]
pub struct ManifestFile {
    pub path: PathBuf,
    pub name: String,
    pub format: ManifestFormat,
    pub kind: ManifestKind,
}

impl ManifestFile {
    fn from_path(path: PathBuf, kind: ManifestKind) -> Self {
        let name = path
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_default();
        let format = if name.ends_with(".json") {
            ManifestFormat::Json
        } else {
            ManifestFormat::Toml
        };
        Self {
            path,
            name,
            format,
            kind,
        }
    }

    pub fn display_path(&self) -> String {
        self.path.to_string_lossy().into_owned()
    }
}

fn manifest_error(code: &str, message: impl Into<String>, path: impl Into<String>) -> NopmError {
    NopmError::with_path(code, message, path)
}

pub fn find_core_manifest(directory: &Path) -> Result<ManifestFile, NopmError> {
    let matches: Vec<&str> = CORE_MANIFEST_NAMES
        .iter()
        .copied()
        .filter(|name| directory.join(name).exists())
        .collect();
    let directory_display = directory.to_string_lossy().into_owned();
    if matches.is_empty() {
        return Err(manifest_error(
            "NS9001",
            "No manifest found. Expected exactly one of core.json or core.toml.",
            directory_display,
        ));
    }
    if matches.len() > 1 {
        return Err(manifest_error(
            "NS9001",
            "Both core.json and core.toml exist. Exactly one manifest is allowed.",
            directory_display,
        ));
    }
    Ok(ManifestFile::from_path(
        directory.join(matches[0]),
        ManifestKind::Core,
    ))
}

pub fn find_build_manifest(directory: &Path) -> Result<Option<ManifestFile>, NopmError> {
    let matches: Vec<&str> = BUILD_MANIFEST_NAMES
        .iter()
        .copied()
        .filter(|name| directory.join(name).exists())
        .collect();
    if matches.len() > 1 {
        return Err(manifest_error(
            "NS9001",
            "Both build.json and build.toml exist. At most one is allowed.",
            directory.to_string_lossy().into_owned(),
        ));
    }
    Ok(matches
        .first()
        .map(|name| ManifestFile::from_path(directory.join(name), ManifestKind::Build)))
}

fn is_record(value: &Value) -> bool {
    value.is_object()
}

pub fn parse_manifest_source(source: &str, file: &ManifestFile) -> Result<Value, NopmError> {
    let normalized = source.trim_start_matches('\u{FEFF}');
    let value = match file.format {
        ManifestFormat::Json => {
            let value = serde_json::from_str::<Value>(normalized).map_err(|error| {
                manifest_error(
                    "NS9002",
                    format!("Invalid JSON manifest: {error}"),
                    file.display_path(),
                )
            })?;
            assert_no_duplicate_json_keys(normalized, file)?;
            value
        }
        ManifestFormat::Toml => {
            let value = toml::from_str::<toml::Value>(normalized).map_err(|error| {
                manifest_error(
                    "NS9002",
                    format!("Invalid TOML manifest: {error}"),
                    file.display_path(),
                )
            })?;
            serde_json::to_value(value).map_err(|error| {
                manifest_error(
                    "NS9002",
                    format!("Invalid TOML manifest: {error}"),
                    file.display_path(),
                )
            })?
        }
    };
    if !is_record(&value) {
        return Err(manifest_error(
            "NS9002",
            "Manifest root must be an object.",
            file.display_path(),
        ));
    }
    Ok(value)
}

pub fn validate_manifest_keys(file: &ManifestFile, data: &Value) -> Result<(), NopmError> {
    let Some(object) = data.as_object() else {
        return Ok(());
    };
    for key in object.keys() {
        let wrong = match file.kind {
            ManifestKind::Core => BUILD_KEYS.contains(&key.as_str()),
            ManifestKind::Build => CORE_KEYS.contains(&key.as_str()),
        };
        if wrong {
            let (owner, _other) = match file.kind {
                ManifestKind::Core => ("build", "core"),
                ManifestKind::Build => ("core", "build"),
            };
            return Err(manifest_error(
                "NS9002",
                format!(
                    "Key '{key}' belongs in {owner}.{}, not {}.",
                    file.format.extension(),
                    file.name
                ),
                file.display_path(),
            ));
        }
    }
    Ok(())
}

pub fn read_manifest(file: &ManifestFile) -> Result<Value, NopmError> {
    let source = fs::read_to_string(&file.path).map_err(|error| {
        manifest_error(
            "NS9002",
            format!("Unable to read manifest: {error}"),
            file.display_path(),
        )
    })?;
    let data = parse_manifest_source(&source, file)?;
    validate_manifest_keys(file, &data)?;
    Ok(data)
}

const CORE_ROOT_ORDER: [&str; 7] = [
    "$schema",
    "package",
    "dependencies",
    "packageSources",
    "registry",
    "ffi",
    "capabilities",
];

const BUILD_ROOT_ORDER: [&str; 8] = [
    "$schema",
    "build",
    "targets",
    "nova",
    "math",
    "strings",
    "memory",
    "toolchain",
];

const PACKAGE_ORDER: [&str; 9] = [
    "name",
    "version",
    "edition",
    "license",
    "description",
    "authors",
    "repository",
    "homepage",
    "keywords",
];

const BUILD_SECTION_ORDER: [&str; 4] = ["defaultTarget", "defaultMode", "profile", "outDir"];
const NOVA_ORDER: [&str; 2] = ["entry", "watch"];
const REGISTRY_ORDER: [&str; 3] = ["default", "defaultSource", "mirrors"];
const MATH_ORDER: [&str; 1] = ["integers"];
const STRINGS_ORDER: [&str; 1] = ["model"];
const MEMORY_ORDER: [&str; 2] = ["default", "files"];
const TARGET_ORDER: [&str; 7] = [
    "backend", "module", "triple", "mode", "outDir", "wasi", "env",
];
const DEPENDENCY_ORDER: [&str; 10] = [
    "source", "version", "path", "git", "ref", "rev", "tag", "targets", "features", "optional",
];
const PACKAGE_SOURCE_ORDER: [&str; 4] = ["enabled", "lifecycleScripts", "registry", "index"];
const PACKAGE_SOURCES_ORDER: [&str; 7] = ["path", "git", "npm", "cargo", "pypi", "nuget", "oci"];
const FFI_TARGET_ORDER: [&str; 3] = ["source", "type", "ownership"];
const CAPABILITY_ORDER: [&str; 3] = ["web", "native", "wasm"];

enum Ordered {
    Scalar(Value),
    Array(Vec<Ordered>),
    Object(Vec<(String, Ordered)>),
}

fn ordered_keys(map: &Map<String, Value>, order: &[&str]) -> Vec<String> {
    let mut keys: Vec<String> = order
        .iter()
        .filter(|key| map.contains_key(**key))
        .map(|key| (*key).to_owned())
        .collect();
    let mut remaining: Vec<String> = map
        .keys()
        .filter(|key| !order.contains(&key.as_str()))
        .cloned()
        .collect();
    remaining.sort_by(|left, right| locale_cmp(left, right));
    keys.extend(remaining);
    keys
}

fn sorted_keys(map: &Map<String, Value>) -> Vec<String> {
    let mut keys: Vec<String> = map.keys().cloned().collect();
    keys.sort_by(|left, right| locale_cmp(left, right));
    keys
}

fn ordered_record(
    map: &Map<String, Value>,
    order: &[&str],
    ffi_order: Option<&[String]>,
) -> Ordered {
    Ordered::Object(
        ordered_keys(map, order)
            .into_iter()
            .map(|key| {
                let value = order_value(&map[&key], Some(&key), ffi_order);
                (key, value)
            })
            .collect(),
    )
}

fn ordered_map(
    map: &Map<String, Value>,
    child_order: &[&str],
    ffi_order: Option<&[String]>,
) -> Ordered {
    Ordered::Object(
        sorted_keys(map)
            .into_iter()
            .map(|key| {
                let value = if map[&key].is_object() {
                    ordered_record(map[&key].as_object().unwrap(), child_order, ffi_order)
                } else {
                    order_value(&map[&key], Some(&key), ffi_order)
                };
                (key, value)
            })
            .collect(),
    )
}

fn ordered_known_map(
    map: &Map<String, Value>,
    outer_order: &[&str],
    child_order: &[&str],
    ffi_order: Option<&[String]>,
) -> Ordered {
    Ordered::Object(
        ordered_keys(map, outer_order)
            .into_iter()
            .map(|key| {
                let value = if map[&key].is_object() {
                    ordered_record(map[&key].as_object().unwrap(), child_order, ffi_order)
                } else {
                    order_value(&map[&key], Some(&key), ffi_order)
                };
                (key, value)
            })
            .collect(),
    )
}

fn order_value(value: &Value, key: Option<&str>, ffi_order: Option<&[String]>) -> Ordered {
    match value {
        Value::Array(items) => Ordered::Array(
            items
                .iter()
                .map(|item| order_value(item, None, ffi_order))
                .collect(),
        ),
        Value::Object(map) => match key {
            None | Some("$schema") => ordered_record(map, &[], ffi_order),
            Some("targets") => ordered_map(map, &TARGET_ORDER, ffi_order),
            Some("dependencies") => ordered_map(map, &DEPENDENCY_ORDER, ffi_order),
            Some("packageSources") => ordered_known_map(
                map,
                &PACKAGE_SOURCES_ORDER,
                &PACKAGE_SOURCE_ORDER,
                ffi_order,
            ),
            Some("ffi") => {
                let mut keys: Vec<String> = Vec::new();
                if let Some(order) = ffi_order {
                    for key in order {
                        if map.contains_key(key) && !keys.contains(key) {
                            keys.push(key.clone());
                        }
                    }
                }
                for key in sorted_keys(map) {
                    if !keys.contains(&key) {
                        keys.push(key);
                    }
                }
                Ordered::Object(
                    keys.into_iter()
                        .map(|key| {
                            let value = if map[&key].is_object() {
                                ordered_record(
                                    map[&key].as_object().unwrap(),
                                    &FFI_TARGET_ORDER,
                                    ffi_order,
                                )
                            } else {
                                order_value(&map[&key], Some(&key), ffi_order)
                            };
                            (key, value)
                        })
                        .collect(),
                )
            }
            Some("capabilities") => ordered_record(map, &CAPABILITY_ORDER, ffi_order),
            Some("memory") => ordered_record(map, &MEMORY_ORDER, ffi_order),
            Some("package") => ordered_record(map, &PACKAGE_ORDER, ffi_order),
            Some("build") => ordered_record(map, &BUILD_SECTION_ORDER, ffi_order),
            Some("nova") => ordered_record(map, &NOVA_ORDER, ffi_order),
            Some("registry") => ordered_record(map, &REGISTRY_ORDER, ffi_order),
            Some("math") => ordered_record(map, &MATH_ORDER, ffi_order),
            Some("strings") => ordered_record(map, &STRINGS_ORDER, ffi_order),
            Some("toolchain") => ordered_record(map, &[], ffi_order),
            Some(_) => ordered_record(map, &[], ffi_order),
        },
        _ => Ordered::Scalar(value.clone()),
    }
}

fn write_ordered(output: &mut String, value: &Ordered, indent: usize) {
    match value {
        Ordered::Scalar(value) => output.push_str(&render_scalar(value)),
        Ordered::Array(items) => {
            if items.is_empty() {
                output.push_str("[]");
                return;
            }
            output.push_str("[\n");
            for (index, item) in items.iter().enumerate() {
                output.push_str(&" ".repeat(indent + 2));
                write_ordered(output, item, indent + 2);
                if index + 1 < items.len() {
                    output.push(',');
                }
                output.push('\n');
            }
            output.push_str(&" ".repeat(indent));
            output.push(']');
        }
        Ordered::Object(entries) => {
            if entries.is_empty() {
                output.push_str("{}");
                return;
            }
            output.push_str("{\n");
            for (index, (key, item)) in entries.iter().enumerate() {
                output.push_str(&" ".repeat(indent + 2));
                output.push_str(&render_scalar(&Value::String(key.clone())));
                output.push_str(": ");
                write_ordered(output, item, indent + 2);
                if index + 1 < entries.len() {
                    output.push(',');
                }
                output.push('\n');
            }
            output.push_str(&" ".repeat(indent));
            output.push('}');
        }
    }
}

fn render_scalar(value: &Value) -> String {
    serde_json::to_string(value).unwrap_or_else(|_| "null".to_owned())
}

fn ffi_key_order(source: &str) -> Option<Vec<String>> {
    let normalized = source.trim_start_matches('\u{FEFF}');
    let mut parser = KeyOrderParser::new(normalized);
    let tree = parser.parse_value().ok()?;
    let KeyTree::Object(entries) = tree else {
        return None;
    };
    let (_, ffi) = entries.into_iter().find(|(key, _)| key == "ffi")?;
    let KeyTree::Object(modules) = ffi else {
        return None;
    };
    Some(modules.into_iter().map(|(key, _)| key).collect())
}

enum KeyParseError {
    Duplicate(String),
    Other,
}

enum KeyTree {
    Object(Vec<(String, KeyTree)>),
    Other,
}

struct KeyOrderParser<'a> {
    bytes: &'a [u8],
    index: usize,
}

impl<'a> KeyOrderParser<'a> {
    fn new(source: &'a str) -> Self {
        Self {
            bytes: source.as_bytes(),
            index: 0,
        }
    }

    fn skip_whitespace(&mut self) {
        while self.index < self.bytes.len() && self.bytes[self.index].is_ascii_whitespace() {
            self.index += 1;
        }
    }

    fn parse_string(&mut self) -> Result<String, KeyParseError> {
        if self.bytes.get(self.index) != Some(&b'"') {
            return Err(KeyParseError::Other);
        }
        let start = self.index;
        self.index += 1;
        while self.index < self.bytes.len() {
            match self.bytes[self.index] {
                b'\\' => self.index += 2,
                b'"' => {
                    self.index += 1;
                    let slice = std::str::from_utf8(&self.bytes[start..self.index])
                        .map_err(|_| KeyParseError::Other)?;
                    return serde_json::from_str(slice).map_err(|_| KeyParseError::Other);
                }
                _ => self.index += 1,
            }
        }
        Err(KeyParseError::Other)
    }

    fn parse_value(&mut self) -> Result<KeyTree, KeyParseError> {
        self.skip_whitespace();
        match self.bytes.get(self.index) {
            Some(b'{') => self.parse_object(),
            Some(b'[') => self.parse_array(),
            Some(b'"') => {
                self.parse_string()?;
                Ok(KeyTree::Other)
            }
            Some(_) => {
                while self.index < self.bytes.len()
                    && !matches!(self.bytes[self.index], b',' | b']' | b'}')
                    && !self.bytes[self.index].is_ascii_whitespace()
                {
                    self.index += 1;
                }
                Ok(KeyTree::Other)
            }
            None => Err(KeyParseError::Other),
        }
    }

    fn parse_array(&mut self) -> Result<KeyTree, KeyParseError> {
        self.index += 1;
        self.skip_whitespace();
        if self.bytes.get(self.index) == Some(&b']') {
            self.index += 1;
            return Ok(KeyTree::Other);
        }
        loop {
            self.parse_value()?;
            self.skip_whitespace();
            match self.bytes.get(self.index) {
                Some(b',') => {
                    self.index += 1;
                    continue;
                }
                Some(b']') => {
                    self.index += 1;
                    return Ok(KeyTree::Other);
                }
                _ => return Err(KeyParseError::Other),
            }
        }
    }

    fn parse_object(&mut self) -> Result<KeyTree, KeyParseError> {
        self.index += 1;
        self.skip_whitespace();
        let mut entries = Vec::new();
        let mut keys = BTreeSet::new();
        if self.bytes.get(self.index) == Some(&b'}') {
            self.index += 1;
            return Ok(KeyTree::Object(entries));
        }
        loop {
            self.skip_whitespace();
            let key = self.parse_string()?;
            if !keys.insert(key.clone()) {
                return Err(KeyParseError::Duplicate(key));
            }
            self.skip_whitespace();
            if self.bytes.get(self.index) != Some(&b':') {
                return Err(KeyParseError::Other);
            }
            self.index += 1;
            let value = self.parse_value()?;
            entries.push((key, value));
            self.skip_whitespace();
            match self.bytes.get(self.index) {
                Some(b',') => {
                    self.index += 1;
                    continue;
                }
                Some(b'}') => {
                    self.index += 1;
                    return Ok(KeyTree::Object(entries));
                }
                _ => return Err(KeyParseError::Other),
            }
        }
    }
}

fn assert_no_duplicate_json_keys(source: &str, file: &ManifestFile) -> Result<(), NopmError> {
    let normalized = source.trim_start_matches('\u{FEFF}');
    let mut parser = KeyOrderParser::new(normalized);
    match parser.parse_value() {
        Ok(_) | Err(KeyParseError::Other) => Ok(()),
        Err(KeyParseError::Duplicate(key)) => Err(manifest_error(
            "NS9002",
            format!("Duplicate key '{key}' in JSON manifest."),
            file.display_path(),
        )),
    }
}

pub fn format_manifest_json(data: &Value, kind: ManifestKind, source: Option<&str>) -> String {
    let ffi_order = source.and_then(ffi_key_order);
    let ordered = match data.as_object() {
        Some(map) => ordered_record(
            map,
            match kind {
                ManifestKind::Core => &CORE_ROOT_ORDER,
                ManifestKind::Build => &BUILD_ROOT_ORDER,
            },
            ffi_order.as_deref(),
        ),
        None => Ordered::Scalar(data.clone()),
    };
    let mut output = String::new();
    write_ordered(&mut output, &ordered, 0);
    output.push('\n');
    output
}

pub fn format_manifest_toml(source: &str, _kind: ManifestKind) -> Result<String, NopmError> {
    let document = source
        .parse::<toml_edit::DocumentMut>()
        .map_err(|error| NopmError::new("NS9002", format!("Invalid TOML manifest: {error}")))?;
    Ok(format!(
        "{}\n",
        document.to_string().replace("\r\n", "\n").trim_end()
    ))
}

fn missing_manifest_error(file: &ManifestFile, error: std::io::Error) -> NopmError {
    manifest_error(
        "NS9002",
        format!("Unable to read manifest: {error}"),
        file.display_path(),
    )
}

pub fn add_dependency(file: &ManifestFile, name: &str, spec: &str) -> Result<(), NopmError> {
    let source =
        fs::read_to_string(&file.path).map_err(|error| missing_manifest_error(file, error))?;
    let data = parse_manifest_source(&source, file)?;
    let mut data = data;
    let mut dependencies = data
        .get("dependencies")
        .and_then(Value::as_object)
        .cloned()
        .unwrap_or_default();
    if dependencies.contains_key(name) {
        return Err(manifest_error(
            "NS9003",
            format!("Dependency '{name}' already exists in {}.", file.name),
            file.display_path(),
        ));
    }
    dependencies.insert(name.to_owned(), Value::String(spec.to_owned()));
    data["dependencies"] = Value::Object(dependencies);

    let formatted = match file.format {
        ManifestFormat::Json => format_manifest_json(&data, file.kind, Some(&source)),
        ManifestFormat::Toml => add_dependency_toml(&source, name, spec, file)?,
    };
    fs::write(&file.path, formatted).map_err(|error| {
        manifest_error(
            "NS9002",
            format!("Unable to write manifest: {error}"),
            file.display_path(),
        )
    })
}

fn add_dependency_toml(
    source: &str,
    name: &str,
    spec: &str,
    file: &ManifestFile,
) -> Result<String, NopmError> {
    let mut document = source.parse::<toml_edit::DocumentMut>().map_err(|error| {
        manifest_error(
            "NS9002",
            format!("Invalid TOML manifest: {error}"),
            file.display_path(),
        )
    })?;
    let had_dependencies = document.get("dependencies").is_some();
    if !had_dependencies {
        let mut table = toml_edit::Table::new();
        if !document.as_table().is_empty() {
            table.decor_mut().set_prefix("\n");
        }
        document.insert("dependencies", toml_edit::Item::Table(table));
    }
    let Some(table) = document
        .get_mut("dependencies")
        .and_then(toml_edit::Item::as_table_mut)
    else {
        return Err(manifest_error(
            "NS9002",
            "Dependencies must be a TOML table.",
            file.display_path(),
        ));
    };
    table.insert(name, toml_edit::value(spec));
    Ok(format!(
        "{}\n",
        document.to_string().replace("\r\n", "\n").trim_end()
    ))
}

pub fn remove_dependency(file: &ManifestFile, name: &str) -> Result<bool, NopmError> {
    let source =
        fs::read_to_string(&file.path).map_err(|error| missing_manifest_error(file, error))?;
    let data = parse_manifest_source(&source, file)?;
    let mut data = data;
    let mut dependencies = data
        .get("dependencies")
        .and_then(Value::as_object)
        .cloned()
        .unwrap_or_default();
    if !dependencies.contains_key(name) {
        return Ok(false);
    }
    dependencies.remove(name);
    if dependencies.is_empty() {
        if let Some(object) = data.as_object_mut() {
            object.remove("dependencies");
        }
    } else {
        data["dependencies"] = Value::Object(dependencies);
    }

    let formatted = match file.format {
        ManifestFormat::Json => format_manifest_json(&data, file.kind, Some(&source)),
        ManifestFormat::Toml => remove_dependency_toml(&source, name, file)?,
    };
    fs::write(&file.path, formatted).map_err(|error| {
        manifest_error(
            "NS9002",
            format!("Unable to write manifest: {error}"),
            file.display_path(),
        )
    })?;
    Ok(true)
}

fn remove_dependency_toml(
    source: &str,
    name: &str,
    file: &ManifestFile,
) -> Result<String, NopmError> {
    let mut document = source.parse::<toml_edit::DocumentMut>().map_err(|error| {
        manifest_error(
            "NS9002",
            format!("Invalid TOML manifest: {error}"),
            file.display_path(),
        )
    })?;
    let mut remove_table = false;
    if let Some(table) = document
        .get_mut("dependencies")
        .and_then(toml_edit::Item::as_table_mut)
    {
        table.remove(name);
        remove_table = table.is_empty();
    }
    if remove_table {
        document.remove("dependencies");
    }
    Ok(format!(
        "{}\n",
        document.to_string().replace("\r\n", "\n").trim_end()
    ))
}
