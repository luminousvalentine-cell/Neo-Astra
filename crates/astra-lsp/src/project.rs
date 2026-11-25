use std::collections::{BTreeMap, HashMap, HashSet};
use std::fs;
use std::path::{Component, Path, PathBuf};

use astra_compiler_api::{
    API_VERSION, CompileRequest, FrontendCompiler, ModuleIr, ProjectCheckRequest,
    ProjectSourceUnit, SourceUnit, check_project,
};
use astra_core::diagnostic::Diagnostic;
use astra_core::edition::Edition;

const MAX_PROJECT_FILES: usize = 2048;
const MAX_PROJECT_DEPTH: usize = 32;

#[derive(Debug, Clone)]
pub struct ProjectModule {
    pub key: String,
    pub path: PathBuf,
    pub uri: String,
    pub text: String,
    pub ir: Option<ModuleIr>,
    pub diagnostics: Vec<Diagnostic>,
}

#[derive(Debug, Clone, Default)]
pub struct ProjectSnapshot {
    pub modules: BTreeMap<String, ProjectModule>,
    by_uri: HashMap<String, String>,
}

impl ProjectSnapshot {
    #[must_use]
    pub fn load(root: &Path, buffers: &HashMap<String, String>) -> Self {
        let root = canonical_path(root);
        let mut files: Vec<PathBuf> = Vec::new();
        let mut seen = HashSet::new();
        collect_directory(&root, &mut files, &mut seen, 0);

        let mut overlay = HashMap::new();
        let mut overlay_uris = HashMap::new();
        for (uri, text) in buffers {
            let Some(path) = uri_to_path(uri) else {
                continue;
            };
            if path.extension().and_then(|ext| ext.to_str()) != Some("ns") {
                continue;
            }
            let path = canonical_path(&path);
            if path.starts_with(&root) {
                let identity = path_identity(&path);
                overlay.insert(identity.clone(), text.clone());
                overlay_uris.insert(identity, uri.clone());
            }
        }

        let mut sources: BTreeMap<String, (PathBuf, String)> = BTreeMap::new();
        for path in files {
            let Some(key) = relative_key(&root, &path) else {
                continue;
            };
            let text = overlay
                .get(&path_identity(&path))
                .cloned()
                .or_else(|| fs::read_to_string(&path).ok())
                .unwrap_or_default();
            sources.insert(key, (path, text));
        }

        for (uri, text) in buffers {
            let Some(path) = uri_to_path(uri) else {
                continue;
            };
            if path.extension().and_then(|ext| ext.to_str()) != Some("ns") {
                continue;
            }
            let path = canonical_path(&path);
            if !path.starts_with(&root) {
                continue;
            }
            let Some(key) = relative_key(&root, &path) else {
                continue;
            };
            sources.entry(key).or_insert_with(|| (path, text.clone()));
        }

        let units: Vec<ProjectSourceUnit> = sources
            .iter()
            .map(|(key, (path, text))| ProjectSourceUnit {
                key: key.clone(),
                path: path.to_string_lossy().into_owned(),
                text: text.clone(),
            })
            .collect();

        let response = check_project(&ProjectCheckRequest {
            api_version: API_VERSION.to_owned(),
            sources: units,
            target: "js".to_owned(),
            edition: Edition::V0_1,
            mode: None,
            integer_policy: Default::default(),
        });

        let mut modules = BTreeMap::new();
        for (key, (path, text)) in sources {
            let identity = path_identity(&path);
            let uri = overlay_uris
                .get(&identity)
                .cloned()
                .unwrap_or_else(|| path_to_uri(&path));
            modules.insert(
                key.clone(),
                ProjectModule {
                    key,
                    uri,
                    path,
                    text,
                    ir: None,
                    diagnostics: Vec::new(),
                },
            );
        }
        for result in response.modules {
            if let Some(module) = modules.get_mut(&result.key) {
                module.diagnostics = result.diagnostics;
                module.ir = result.ir;
            }
        }

        for module in modules.values_mut() {
            if module.ir.is_some() {
                continue;
            }
            let response = FrontendCompiler.compile(&CompileRequest {
                api_version: API_VERSION.to_owned(),
                source: SourceUnit {
                    path: module.key.clone(),
                    text: module.text.clone(),
                },
                target: "js".to_owned(),
                edition: Edition::V0_1,
                mode: None,
                integer_policy: Default::default(),
            });
            module.ir = response.ir;
        }

        let mut snapshot = Self {
            modules,
            by_uri: HashMap::new(),
        };
        snapshot.rebuild_uri_index();
        snapshot
    }

    #[must_use]
    pub fn single(uri: &str, text: &str) -> Self {
        let path = uri_to_path(uri).unwrap_or_else(|| PathBuf::from("buffer.ns"));
        let path = canonical_path(&path);
        let key = path
            .strip_prefix(path.parent().unwrap_or(Path::new("")))
            .ok()
            .and_then(|relative| relative.to_str())
            .map(|relative| relative.replace('\\', "/"))
            .filter(|relative| !relative.is_empty())
            .unwrap_or_else(|| "buffer.ns".to_owned());

        let compiler = FrontendCompiler;
        let response = compiler.compile(&CompileRequest {
            api_version: API_VERSION.to_owned(),
            source: SourceUnit {
                path: key.clone(),
                text: text.to_owned(),
            },
            target: "js".to_owned(),
            edition: Edition::V0_1,
            mode: None,
            integer_policy: Default::default(),
        });

        let mut modules = BTreeMap::new();
        modules.insert(
            key.clone(),
            ProjectModule {
                key,
                path: path.clone(),
                uri: if uri.starts_with("file:") {
                    uri.to_owned()
                } else {
                    path_to_uri(&path)
                },
                text: text.to_owned(),
                ir: response.ir,
                diagnostics: response.diagnostics,
            },
        );
        let mut snapshot = Self {
            modules,
            by_uri: HashMap::new(),
        };
        snapshot.rebuild_uri_index();
        snapshot
    }

    fn rebuild_uri_index(&mut self) {
        self.by_uri.clear();
        for (key, module) in &self.modules {
            self.by_uri.insert(module.uri.clone(), key.clone());
        }
    }

    #[must_use]
    pub fn module_for_uri(&self, uri: &str) -> Option<&ProjectModule> {
        if let Some(key) = self.by_uri.get(uri) {
            return self.modules.get(key);
        }
        let path = uri_to_path(uri)?;
        let identity = path_identity(&canonical_path(&path));
        self.modules
            .values()
            .find(|module| path_identity(&module.path) == identity)
    }

    #[must_use]
    pub fn module(&self, key: &str) -> Option<&ProjectModule> {
        self.modules.get(key)
    }
}

#[must_use]
pub fn uri_to_path(uri: &str) -> Option<PathBuf> {
    let rest = uri.strip_prefix("file://")?;
    let decoded = percent_decode(rest)?;

    let path = if let Some(path) = decoded.strip_prefix('/') {
        if cfg!(windows)
            && path.as_bytes().get(1) == Some(&b':')
            && path
                .as_bytes()
                .first()
                .is_some_and(|byte| byte.is_ascii_alphabetic())
        {
            path.to_owned()
        } else {
            format!("/{path}")
        }
    } else if decoded.is_empty() {
        String::new()
    } else {
        format!("//{decoded}")
    };

    if path.is_empty() {
        None
    } else {
        Some(PathBuf::from(path))
    }
}

#[must_use]
pub fn path_to_uri(path: &Path) -> String {
    let path = canonical_path(path);
    let mut text = path.to_string_lossy().replace('\\', "/");
    if cfg!(windows) && text.starts_with("//") {
        text = format!("file:{}", percent_encode(&text));
    } else if text.starts_with('/') {
        text = format!("file://{}", percent_encode(&text));
    } else {
        text = format!("file:///{}", percent_encode(&text));
    }
    text
}

fn percent_decode(input: &str) -> Option<String> {
    let bytes = input.as_bytes();
    let mut decoded = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] == b'%' {
            let high = hex_value(*bytes.get(index + 1)?)?;
            let low = hex_value(*bytes.get(index + 2)?)?;
            decoded.push((high << 4) | low);
            index += 3;
        } else {
            decoded.push(bytes[index]);
            index += 1;
        }
    }
    String::from_utf8(decoded).ok()
}

fn hex_value(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        b'A'..=b'F' => Some(byte - b'A' + 10),
        _ => None,
    }
}

fn percent_encode(input: &str) -> String {
    let mut output = String::with_capacity(input.len());
    for byte in input.bytes() {
        if byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'.' | b'_' | b'~' | b'/' | b':') {
            output.push(char::from(byte));
        } else {
            output.push('%');
            output.push(char::from(HEX[(byte >> 4) as usize]));
            output.push(char::from(HEX[(byte & 0x0f) as usize]));
        }
    }
    output
}

const HEX: &[u8; 16] = b"0123456789ABCDEF";

fn collect_directory(
    directory: &Path,
    files: &mut Vec<PathBuf>,
    seen: &mut HashSet<String>,
    depth: usize,
) {
    if depth > MAX_PROJECT_DEPTH || files.len() >= MAX_PROJECT_FILES {
        return;
    }
    let Ok(entries) = fs::read_dir(directory) else {
        return;
    };
    let mut entries: Vec<_> = entries.flatten().collect();
    entries.sort_by_key(|entry| entry.file_name());

    for entry in entries {
        if files.len() >= MAX_PROJECT_FILES {
            return;
        }
        let name = entry.file_name();
        let name = name.to_string_lossy();
        if name.starts_with('.') {
            continue;
        }
        let Ok(file_type) = entry.file_type() else {
            continue;
        };
        let path = entry.path();
        if file_type.is_dir() {
            if should_skip_directory(&name) {
                continue;
            }
            collect_directory(&path, files, seen, depth + 1);
        } else if file_type.is_file()
            && path.extension().and_then(|ext| ext.to_str()) == Some("ns")
            && seen.insert(path_identity(&path))
        {
            files.push(path);
        }
    }
}

fn should_skip_directory(name: &str) -> bool {
    matches!(
        name,
        "target" | "node_modules" | "dist" | "build" | "out" | ".git"
    )
}

fn relative_key(root: &Path, path: &Path) -> Option<String> {
    let relative = path.strip_prefix(root).ok()?;
    let mut parts = Vec::new();
    for component in relative.components() {
        match component {
            Component::Normal(part) => parts.push(part.to_string_lossy().into_owned()),
            Component::CurDir => {}
            _ => return None,
        }
    }
    if parts.is_empty() {
        None
    } else {
        Some(parts.join("/"))
    }
}

fn canonical_path(path: &Path) -> PathBuf {
    if let Ok(canonical) = fs::canonicalize(path) {
        return clean_windows_path(canonical);
    }
    let absolute = if path.is_absolute() {
        path.to_path_buf()
    } else {
        std::env::current_dir()
            .unwrap_or_else(|_| PathBuf::from("."))
            .join(path)
    };
    let Some(parent) = absolute.parent() else {
        return clean_windows_path(absolute);
    };
    let Some(name) = absolute.file_name() else {
        return clean_windows_path(absolute);
    };
    clean_windows_path(
        fs::canonicalize(parent)
            .unwrap_or_else(|_| parent.to_path_buf())
            .join(name),
    )
}

fn clean_windows_path(path: PathBuf) -> PathBuf {
    #[cfg(windows)]
    {
        let text = path.to_string_lossy();
        if let Some(rest) = text.strip_prefix(r"\\?\UNC\") {
            return PathBuf::from(format!(r"\\{rest}"));
        }
        if let Some(rest) = text.strip_prefix(r"\\?\") {
            return PathBuf::from(rest);
        }
    }
    path
}

fn path_identity(path: &Path) -> String {
    let text = path.to_string_lossy().replace('\\', "/");
    if cfg!(windows) {
        text.to_ascii_lowercase()
    } else {
        text
    }
}
