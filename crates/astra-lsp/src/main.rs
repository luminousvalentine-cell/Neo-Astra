use std::collections::HashMap;
use std::io::{self, BufRead, Write};
use std::process;

use astra_compiler_api::{API_VERSION, CompileRequest, CompilerApi, FrontendCompiler, SourceUnit};
use astra_core::diagnostic::{Diagnostic, Severity};
use astra_core::formatter::format_source;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

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
struct LspDiagnostic {
    range: Range,
    severity: u32,
    code: String,
    source: String,
    message: String,
}

struct Server {
    buffers: HashMap<String, String>,
    shutdown: bool,
}

impl Server {
    fn new() -> Self {
        Self {
            buffers: HashMap::new(),
            shutdown: false,
        }
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
                let result = json!({
                    "capabilities": {
                        "textDocumentSync": 1,
                        "documentFormattingProvider": true
                    },
                    "serverInfo": {
                        "name": "astra-lsp",
                        "version": "1.0.0"
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
                        publish_diagnostics(&server, &uri, &text);
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
                        publish_diagnostics(&server, &uri, &text);
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
                    publish_diagnostics(&server, &uri, &text);
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
                    }
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

fn publish_diagnostics(_server: &Server, uri: &str, text: &str) {
    let diags = check_buffer(text);
    let lsp_diags: Vec<LspDiagnostic> = diags.iter().map(|d| diag_to_lsp(text, d)).collect();
    write_message(&json!({
        "jsonrpc": "2.0",
        "method": "textDocument/publishDiagnostics",
        "params": {
            "uri": uri,
            "diagnostics": lsp_diags
        }
    }));
}

fn publish_diagnostics_for_text(uri: &str, text: &str, diags: &[Diagnostic]) {
    let lsp_diags: Vec<LspDiagnostic> = diags.iter().map(|d| diag_to_lsp(text, d)).collect();
    write_message(&json!({
        "jsonrpc": "2.0",
        "method": "textDocument/publishDiagnostics",
        "params": {
            "uri": uri,
            "diagnostics": lsp_diags
        }
    }));
}

fn check_buffer(text: &str) -> Vec<Diagnostic> {
    let request = CompileRequest {
        api_version: API_VERSION.to_owned(),
        source: SourceUnit {
            path: "buffer.ns".to_owned(),
            text: text.to_owned(),
        },
        target: "js".to_owned(),
        mode: None,
        integer_policy: Default::default(),
    };
    let compiler = FrontendCompiler;
    let response = compiler.check(&request);
    response.diagnostics
}

fn diag_to_lsp(text: &str, d: &Diagnostic) -> LspDiagnostic {
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
    }
}

fn offset_to_position(text: &str, offset: usize) -> (u32, u32) {
    let offset = offset.min(text.len());

    let mut o = offset;
    while o > 0 && !text.is_char_boundary(o) {
        o -= 1;
    }
    let mut line = 0u32;
    let mut col = 0u32;
    let mut i = 0usize;
    for ch in text.chars() {
        if i >= o {
            break;
        }
        let len = ch.len_utf8();
        if i + len > o {
            break;
        }
        if ch == '\n' {
            line += 1;
            col = 0;
        } else {
            col += ch.len_utf16() as u32;
        }
        i += len;
    }
    (line, col)
}
