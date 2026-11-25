use std::collections::HashMap;
use std::io::{self, BufRead, Write};
use std::path::PathBuf;
use std::process;

use astra_core::diagnostic::{Diagnostic, Severity};
use astra_core::formatter::format_source;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

mod features;
mod project;
mod text;

use project::{ProjectSnapshot, uri_to_path};
use text::offset_to_position;

#[derive(Deserialize)]
struct RpcMessage {
    #[serde(default)]
    id: Option<Value>,
    #[serde(default)]
    method: String,
    #[serde(default)]
    params: Option<Value>,
}

#[derive(Serialize)]
struct Position {
    line: u32,
    character: u32,
}

#[derive(Serialize)]
struct Range {
    start: Position,
    end: Position,
}

#[derive(Serialize)]
struct TextEdit {
    range: Range,
    #[serde(rename = "newText")]
    new_text: String,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct LspLocation {
    uri: String,
    range: Range,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct LspRelatedInformation {
    location: LspLocation,
    message: String,
}

#[derive(Serialize)]
struct LspDiagnostic {
    range: Range,
    severity: u32,
    code: String,
    source: String,
    message: String,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    related_information: Vec<LspRelatedInformation>,
}

struct Server {
    buffers: HashMap<String, String>,
    root: Option<PathBuf>,
    project: Option<ProjectSnapshot>,
    shutdown: bool,
}

impl Server {
    fn new() -> Self {
        Self {
            buffers: HashMap::new(),
            root: None,
            project: None,
            shutdown: false,
        }
    }

    fn refresh_project(&mut self, uri: &str) {
        let Some(root) = self.root.clone() else {
            if let Some(text) = self.buffers.get(uri) {
                let snapshot = ProjectSnapshot::single(uri, text);
                publish_project_diagnostics(&snapshot);
                self.project = Some(snapshot);
            }
            return;
        };
        let snapshot = ProjectSnapshot::load(&root, &self.buffers);
        publish_project_diagnostics(&snapshot);
        self.project = Some(snapshot);
    }
}

fn main() {
    let stdin = io::stdin();
    let mut reader = stdin.lock();
    let mut server = Server::new();
    loop {
        let msg = match read_message(&mut reader) {
            Ok(Some(m)) => m,
            Ok(None) => break,
            Err(_) => {
                write_message(&json!({
                    "jsonrpc": "2.0",
                    "id": null,
                    "error": { "code": -32700, "message": "Parse error" }
                }));
                continue;
            }
        };

        let is_notification = msg.id.is_none();

        match msg.method.as_str() {
            "initialize" => {
                if let Some(params) = &msg.params {
                    server.root = workspace_root(params);
                }
                let result = json!({
                    "capabilities": {
                        "textDocumentSync": 1,
                        "documentFormattingProvider": true,
                        "hoverProvider": true,
                        "definitionProvider": true,
                        "referencesProvider": true,
                        "documentHighlightProvider": true,
                        "documentSymbolProvider": true,
                        "workspaceSymbolProvider": true,
                        "renameProvider": true,
                        "signatureHelpProvider": {
                            "triggerCharacters": ["(", ","]
                        },
                        "completionProvider": {
                            "triggerCharacters": [".", ":"]
                        }
                    },
                    "serverInfo": {
                        "name": "astra-lsp",
                        "version": "2.0.0"
                    }
                });
                if let Some(id) = msg.id {
                    write_message(&json!({
                        "jsonrpc": "2.0",
                        "id": id,
                        "result": result
                    }));
                }
            }
            "shutdown" => {
                server.shutdown = true;
                if let Some(id) = msg.id {
                    write_message(&json!({
                        "jsonrpc": "2.0",
                        "id": id,
                        "result": null
                    }));
                }
            }
            "exit" => {
                if server.shutdown {
                    process::exit(0);
                } else {
                    process::exit(1);
                }
            }
            "initialized" => {}
            "textDocument/didOpen" => {
                if let Some(params) = msg.params
                    && let Some(td) = params.get("textDocument")
                {
                    let uri = td
                        .get("uri")
                        .and_then(|v| v.as_str())
                        .unwrap_or("")
                        .to_owned();
                    let text = td
                        .get("text")
                        .and_then(|v| v.as_str())
                        .unwrap_or("")
                        .to_owned();
                    if !uri.is_empty() {
                        server.buffers.insert(uri.clone(), text.clone());
                        server.refresh_project(&uri);
                    }
                }
            }
            "textDocument/didChange" => {
                if let Some(params) = msg.params {
                    let uri = params
                        .get("textDocument")
                        .and_then(|td| td.get("uri"))
                        .and_then(|v| v.as_str())
                        .unwrap_or("")
                        .to_owned();
                    if uri.is_empty() {
                        continue;
                    }

                    let mut new_text: Option<String> = None;
                    if let Some(changes) = params.get("contentChanges").and_then(|v| v.as_array()) {
                        for change in changes {
                            if let Some(t) = change.get("text").and_then(|v| v.as_str()) {
                                new_text = Some(t.to_owned());
                            }
                        }
                    }
                    if let Some(text) = new_text {
                        server.buffers.insert(uri.clone(), text.clone());
                        server.refresh_project(&uri);
                    }
                }
            }
            "textDocument/didSave" => {
                if let Some(params) = msg.params {
                    let uri = params
                        .get("textDocument")
                        .and_then(|td| td.get("uri"))
                        .and_then(|v| v.as_str())
                        .unwrap_or("")
                        .to_owned();
                    if uri.is_empty() {
                        continue;
                    }
                    let text_opt = params
                        .get("text")
                        .and_then(|v| v.as_str())
                        .map(|s| s.to_owned());
                    let text = if let Some(t) = text_opt {
                        t
                    } else {
                        server.buffers.get(&uri).cloned().unwrap_or_default()
                    };
                    server.buffers.insert(uri.clone(), text.clone());
                    server.refresh_project(&uri);
                }
            }
            "textDocument/didClose" => {
                if let Some(params) = msg.params {
                    let uri = params
                        .get("textDocument")
                        .and_then(|td| td.get("uri"))
                        .and_then(|v| v.as_str())
                        .unwrap_or("")
                        .to_owned();
                    if !uri.is_empty() {
                        server.buffers.remove(&uri);

                        write_message(&json!({
                            "jsonrpc": "2.0",
                            "method": "textDocument/publishDiagnostics",
                            "params": {
                                "uri": uri,
                                "diagnostics": []
                            }
                        }));
                        if server.root.is_some() {
                            server.refresh_project(&uri);
                        }
                    }
                }
            }
            "textDocument/hover" => {
                if let Some(id) = msg.id.clone() {
                    let (uri, line, character) = request_position(msg.params.as_ref());
                    let result = with_snapshot(&server, &uri, |snapshot| {
                        features::hover(snapshot, &uri, line, character)
                    })
                    .flatten();
                    write_message(&json!({
                        "jsonrpc": "2.0",
                        "id": id,
                        "result": result
                    }));
                }
            }
            "textDocument/definition" => {
                if let Some(id) = msg.id.clone() {
                    let (uri, line, character) = request_position(msg.params.as_ref());
                    let result = with_snapshot(&server, &uri, |snapshot| {
                        features::definition(snapshot, &uri, line, character)
                    })
                    .flatten();
                    write_message(&json!({
                        "jsonrpc": "2.0",
                        "id": id,
                        "result": result
                    }));
                }
            }
            "textDocument/references" => {
                if let Some(id) = msg.id.clone() {
                    let (uri, line, character) = request_position(msg.params.as_ref());
                    let include_declaration = msg
                        .params
                        .as_ref()
                        .and_then(|params| params.get("context"))
                        .and_then(|context| context.get("includeDeclaration"))
                        .and_then(Value::as_bool)
                        .unwrap_or(true);
                    let result = with_snapshot(&server, &uri, |snapshot| {
                        features::references(snapshot, &uri, line, character, include_declaration)
                    })
                    .unwrap_or_default();
                    write_message(&json!({
                        "jsonrpc": "2.0",
                        "id": id,
                        "result": result
                    }));
                }
            }
            "textDocument/documentHighlight" => {
                if let Some(id) = msg.id.clone() {
                    let (uri, line, character) = request_position(msg.params.as_ref());
                    let result = with_snapshot(&server, &uri, |snapshot| {
                        features::document_highlight(snapshot, &uri, line, character)
                    })
                    .unwrap_or_default();
                    write_message(&json!({
                        "jsonrpc": "2.0",
                        "id": id,
                        "result": result
                    }));
                }
            }
            "textDocument/rename" => {
                if let Some(id) = msg.id.clone() {
                    let (uri, line, character) = request_position(msg.params.as_ref());
                    let new_name = msg
                        .params
                        .as_ref()
                        .and_then(|params| params.get("newName"))
                        .and_then(Value::as_str)
                        .unwrap_or("")
                        .to_owned();
                    let result = with_snapshot(&server, &uri, |snapshot| {
                        features::rename(snapshot, &uri, line, character, &new_name)
                    })
                    .flatten();
                    write_message(&json!({
                        "jsonrpc": "2.0",
                        "id": id,
                        "result": result
                    }));
                }
            }
            "textDocument/completion" => {
                if let Some(id) = msg.id.clone() {
                    let (uri, line, character) = request_position(msg.params.as_ref());
                    let result = with_snapshot(&server, &uri, |snapshot| {
                        features::completion(snapshot, &uri, line, character)
                    })
                    .unwrap_or_default();
                    write_message(&json!({
                        "jsonrpc": "2.0",
                        "id": id,
                        "result": result
                    }));
                }
            }
            "textDocument/documentSymbol" => {
                if let Some(id) = msg.id.clone() {
                    let uri = msg
                        .params
                        .as_ref()
                        .and_then(|params| params.get("textDocument"))
                        .and_then(|document| document.get("uri"))
                        .and_then(Value::as_str)
                        .unwrap_or("")
                        .to_owned();
                    let result = with_snapshot(&server, &uri, |snapshot| {
                        features::document_symbols(snapshot, &uri)
                    })
                    .unwrap_or_default();
                    write_message(&json!({
                        "jsonrpc": "2.0",
                        "id": id,
                        "result": result
                    }));
                }
            }
            "workspace/symbol" => {
                if let Some(id) = msg.id.clone() {
                    let query = msg
                        .params
                        .as_ref()
                        .and_then(|params| params.get("query"))
                        .and_then(Value::as_str)
                        .unwrap_or("")
                        .to_owned();
                    let result = server.project.as_ref().map_or_else(Vec::new, |snapshot| {
                        features::workspace_symbols(snapshot, &query)
                    });
                    write_message(&json!({
                        "jsonrpc": "2.0",
                        "id": id,
                        "result": result
                    }));
                }
            }
            "textDocument/signatureHelp" => {
                if let Some(id) = msg.id.clone() {
                    let (uri, line, character) = request_position(msg.params.as_ref());
                    let result = with_snapshot(&server, &uri, |snapshot| {
                        features::signature_help(snapshot, &uri, line, character)
                    })
                    .flatten();
                    write_message(&json!({
                        "jsonrpc": "2.0",
                        "id": id,
                        "result": result
                    }));
                }
            }
            "textDocument/formatting" => {
                if let Some(id) = msg.id.clone() {
                    let uri = msg
                        .params
                        .as_ref()
                        .and_then(|p| p.get("textDocument"))
                        .and_then(|td| td.get("uri"))
                        .and_then(|v| v.as_str())
                        .unwrap_or("")
                        .to_owned();

                    let edits = if let Some(text) = server.buffers.get(&uri) {
                        match format_source(text) {
                            Ok(formatted) => {
                                if formatted != *text {
                                    let end = offset_to_position(text, text.len());
                                    vec![TextEdit {
                                        range: Range {
                                            start: Position {
                                                line: 0,
                                                character: 0,
                                            },
                                            end: Position {
                                                line: end.0,
                                                character: end.1,
                                            },
                                        },
                                        new_text: formatted,
                                    }]
                                } else {
                                    Vec::new()
                                }
                            }
                            Err(diags) => {
                                publish_diagnostics_for_text(&uri, text, &diags);
                                Vec::new()
                            }
                        }
                    } else {
                        Vec::new()
                    };

                    write_message(&json!({
                        "jsonrpc": "2.0",
                        "id": id,
                        "result": edits
                    }));
                }
            }
            _other => {
                if is_notification {
                } else if let Some(id) = msg.id {
                    write_message(&json!({
                        "jsonrpc": "2.0",
                        "id": id,
                        "error": { "code": -32601, "message": "Method not found" }
                    }));
                }
            }
        }
    }
}

fn workspace_root(params: &Value) -> Option<PathBuf> {
    if let Some(root_uri) = params.get("rootUri").and_then(Value::as_str)
        && let Some(path) = uri_to_path(root_uri)
    {
        return Some(root_directory(path));
    }
    if let Some(root_path) = params.get("rootPath").and_then(Value::as_str) {
        return Some(root_directory(PathBuf::from(root_path)));
    }
    params
        .get("workspaceFolders")
        .and_then(Value::as_array)
        .and_then(|folders| folders.first())
        .and_then(|folder| folder.get("uri"))
        .and_then(Value::as_str)
        .and_then(uri_to_path)
        .map(root_directory)
}

fn root_directory(path: PathBuf) -> PathBuf {
    if path.is_file() {
        path.parent().map_or(path.clone(), PathBuf::from)
    } else {
        path
    }
}

fn request_position(params: Option<&Value>) -> (String, u32, u32) {
    let params = params.unwrap_or(&Value::Null);
    let uri = params
        .get("textDocument")
        .and_then(|document| document.get("uri"))
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_owned();
    let line = params
        .get("position")
        .and_then(|position| position.get("line"))
        .and_then(Value::as_u64)
        .unwrap_or(0) as u32;
    let character = params
        .get("position")
        .and_then(|position| position.get("character"))
        .and_then(Value::as_u64)
        .unwrap_or(0) as u32;
    (uri, line, character)
}

fn with_snapshot<R>(
    server: &Server,
    uri: &str,
    feature: impl FnOnce(&ProjectSnapshot) -> R,
) -> Option<R> {
    if let Some(snapshot) = &server.project
        && snapshot.module_for_uri(uri).is_some()
    {
        return Some(feature(snapshot));
    }
    let text = server.buffers.get(uri)?;
    let snapshot = ProjectSnapshot::single(uri, text);
    Some(feature(&snapshot))
}

fn publish_project_diagnostics(snapshot: &ProjectSnapshot) {
    for module in snapshot.modules.values() {
        let diagnostics: Vec<LspDiagnostic> = module
            .diagnostics
            .iter()
            .map(|diagnostic| diag_to_lsp(&module.text, diagnostic, Some(&module.uri)))
            .collect();
        write_message(&json!({
            "jsonrpc": "2.0",
            "method": "textDocument/publishDiagnostics",
            "params": {
                "uri": module.uri,
                "diagnostics": diagnostics
            }
        }));
    }
}

fn read_message(reader: &mut impl BufRead) -> Result<Option<RpcMessage>, ()> {
    let mut content_length: Option<usize> = None;
    loop {
        let mut line = String::new();
        let bytes = reader.read_line(&mut line).map_err(|_| ())?;
        if bytes == 0 {
            if content_length.is_none() {
                return Ok(None);
            }
            return Err(());
        }
        let trimmed = line.trim_end_matches(['\r', '\n']);
        if trimmed.is_empty() {
            break;
        }
        if let Some((name, value)) = trimmed.split_once(':')
            && name.trim().eq_ignore_ascii_case("content-length")
        {
            content_length = value.trim().parse().ok();
        }
    }
    let len = content_length.ok_or(())?;
    let mut buf = vec![0u8; len];
    reader.read_exact(&mut buf).map_err(|_| ())?;
    let text = String::from_utf8(buf).map_err(|_| ())?;

    let value: Value = serde_json::from_str(&text).map_err(|_| ())?;

    let msg: RpcMessage = serde_json::from_value(value).map_err(|_| ())?;
    Ok(Some(msg))
}

fn write_message(value: &Value) {
    let body = serde_json::to_string(value).unwrap_or_default();
    let mut stdout = io::stdout().lock();
    let _ = write!(stdout, "Content-Length: {}\r\n\r\n{}", body.len(), body);
    let _ = stdout.flush();
}

fn publish_diagnostics_for_text(uri: &str, text: &str, diags: &[Diagnostic]) {
    let lsp_diags: Vec<LspDiagnostic> = diags
        .iter()
        .map(|d| diag_to_lsp(text, d, Some(uri)))
        .collect();
    write_message(&json!({
        "jsonrpc": "2.0",
        "method": "textDocument/publishDiagnostics",
        "params": {
            "uri": uri,
            "diagnostics": lsp_diags
        }
    }));
}

fn diag_to_lsp(text: &str, d: &Diagnostic, uri: Option<&str>) -> LspDiagnostic {
    let (start, end) = if let Some(span) = d.span {
        (
            offset_to_position(text, span.start),
            offset_to_position(text, span.end),
        )
    } else {
        ((0, 0), (0, 0))
    };
    let severity = match d.severity {
        Severity::Error => 1,
        Severity::Warning => 2,
        Severity::Note => 3,
        Severity::Help => 4,
    };
    let message = if let Some(hint) = &d.hint {
        format!("{} {}", d.message, hint)
    } else {
        d.message.clone()
    };
    let related_information = match uri {
        Some(uri) => d
            .related
            .iter()
            .filter(|entry| entry.file.is_empty())
            .map(|entry| LspRelatedInformation {
                location: LspLocation {
                    uri: uri.to_owned(),
                    range: Range {
                        start: Position {
                            line: offset_to_position(text, entry.span.start).0,
                            character: offset_to_position(text, entry.span.start).1,
                        },
                        end: Position {
                            line: offset_to_position(text, entry.span.end).0,
                            character: offset_to_position(text, entry.span.end).1,
                        },
                    },
                },
                message: entry.message.clone(),
            })
            .collect(),
        None => Vec::new(),
    };
    LspDiagnostic {
        range: Range {
            start: Position {
                line: start.0,
                character: start.1,
            },
            end: Position {
                line: end.0,
                character: end.1,
            },
        },
        severity,
        code: d.code.clone(),
        source: "astra".to_owned(),
        message,
        related_information,
    }
}
