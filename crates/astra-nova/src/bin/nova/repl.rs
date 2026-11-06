use std::io::{self, Write};

use astra_core::ast;
use astra_core::mode::Mode;
use astra_core::parser::parse;
use astra_nova::{NovaError, NovaOptions, resolve_source_mode};
use serde_json::{Value as JsonValue, json};

use crate::{captured_call, emit_json_report, rendered_value};

const HELP: &str = "\
Commands:
  :help, :h   Show this help
  :quit, :q   Exit the session

Enter declarations, `let` bindings, assignments, or expressions.
Declarations, `let` bindings, and assignments persist across lines.
Input is line-oriented; enter one statement per line.
";

#[derive(Default)]
struct Session {
    declarations: Vec<String>,
    state: Vec<String>,
}

enum InputKind {
    Declaration,
    State,
    Expression,
}

struct Outcome {
    result: Result<Option<String>, NovaError>,
    stdout: String,
    stderr: String,
    mode: Mode,
}

pub(crate) fn run(mode: Mode, infer_mode: bool, json: bool) -> i32 {
    let stdin = io::stdin();
    let mut session = Session::default();

    if !json {
        println!("Nova REPL. Type :help for commands.");
    }

    loop {
        if !json {
            print!("nova> ");
            let _ = io::stdout().flush();
        }

        let mut line = String::new();
        match stdin.read_line(&mut line) {
            Ok(0) => {
                if !json {
                    println!();
                }
                break;
            }
            Ok(_) => {}
            Err(error) => {
                eprintln!("NS9008: Unable to read input: {error}");
                return 1;
            }
        }

        let input = line.trim();
        if input.is_empty() {
            continue;
        }

        match input {
            ":quit" | ":q" => break,
            ":help" | ":h" => {
                if json {
                    emit_json_report(true, Some("repl"), mode, None, HELP, "", Vec::new());
                } else {
                    print!("{HELP}");
                }
                continue;
            }
            _ => {}
        }

        let kind = classify(input);
        let outcome = session.submit(input, kind, mode, infer_mode);

        if json {
            let (ok, value, diagnostics) = match &outcome.result {
                Ok(value) => (true, value.as_deref(), Vec::new()),
                Err(error) => (false, None, json_diagnostics(error)),
            };
            emit_json_report(
                ok,
                Some("repl"),
                outcome.mode,
                value,
                &outcome.stdout,
                &outcome.stderr,
                diagnostics,
            );
        } else {
            match outcome.result {
                Ok(value) => {
                    print!("{}", outcome.stdout);
                    eprint!("{}", outcome.stderr);
                    if let Some(value) = value {
                        println!("{value}");
                    }
                }
                Err(error) => {
                    print!("{}", outcome.stdout);
                    eprint!("{}", outcome.stderr);
                    print_error(&error);
                }
            }
        }
    }

    0
}

impl Session {
    fn submit(&mut self, input: &str, kind: InputKind, mode: Mode, infer_mode: bool) -> Outcome {
        let options = NovaOptions {
            mode,
            infer_mode,
            ..NovaOptions::default()
        };

        match kind {
            InputKind::Declaration => {
                self.declarations.push(input.to_owned());
                let source = self.declaration_source();
                let (options, mode) = resolve_options(&source, options);
                let (result, _stdout, _stderr) =
                    captured_call(&source, options, Some("__nova_repl"));
                match result {
                    Ok(_) => Outcome {
                        result: Ok(None),
                        stdout: String::new(),
                        stderr: String::new(),
                        mode,
                    },
                    Err(error) => {
                        self.declarations.pop();
                        Outcome {
                            result: Err(error),
                            stdout: String::new(),
                            stderr: String::new(),
                            mode,
                        }
                    }
                }
            }
            InputKind::State => {
                self.state.push(input.to_owned());
                let source = self.execution_source(None);
                let (options, mode) = resolve_options(&source, options);
                let (result, stdout, stderr) = captured_call(&source, options, Some("__nova_repl"));
                match result {
                    Ok(_) => Outcome {
                        result: Ok(None),
                        stdout: String::new(),
                        stderr: String::new(),
                        mode,
                    },
                    Err(error) => {
                        self.state.pop();
                        Outcome {
                            result: Err(error),
                            stdout,
                            stderr,
                            mode,
                        }
                    }
                }
            }
            InputKind::Expression => {
                let source = self.execution_source(Some(input));
                let (options, mode) = resolve_options(&source, options);
                let (result, stdout, stderr) = captured_call(&source, options, Some("__nova_repl"));
                let result = result.map(|value| rendered_value(&value));
                Outcome {
                    result,
                    stdout,
                    stderr,
                    mode,
                }
            }
        }
    }

    fn declaration_source(&self) -> String {
        let mut source = self.declarations.join("\n");
        source.push_str("\nfn __nova_repl() {}\n");
        source
    }

    fn execution_source(&self, input: Option<&str>) -> String {
        let mut source = self.declarations.join("\n");
        source.push_str("\nfn __nova_repl() {\n");
        for statement in &self.state {
            source.push_str(statement);
            source.push('\n');
        }
        if let Some(input) = input {
            source.push_str(input);
            source.push('\n');
        }
        source.push_str("}\n");
        source
    }
}

fn classify(input: &str) -> InputKind {
    if let Ok(module) = parse(input)
        && !module.items.is_empty()
    {
        return InputKind::Declaration;
    }

    if is_state_statement(input) {
        InputKind::State
    } else {
        InputKind::Expression
    }
}

fn is_state_statement(input: &str) -> bool {
    let wrapped = format!("fn __probe() {{\n{input}\n}}\n");
    let Ok(module) = parse(&wrapped) else {
        return false;
    };

    let Some(function) = module.items.iter().find_map(|item| match &item.kind {
        ast::ItemKind::Function(function) if function.name == "__probe" => Some(function),
        _ => None,
    }) else {
        return false;
    };

    if function.body.statements.len() != 1 {
        return false;
    }

    match &function.body.statements[0].kind {
        ast::StmtKind::Let { .. } => true,
        ast::StmtKind::Expr(expression) => {
            matches!(expression.kind, ast::ExprKind::Assign { .. })
        }
        _ => false,
    }
}

fn resolve_options(source: &str, options: NovaOptions) -> (NovaOptions, Mode) {
    match resolve_source_mode(source, options) {
        Ok(mode) => (
            NovaOptions {
                mode,
                infer_mode: false,
                ..options
            },
            mode,
        ),
        Err(_) => (options, options.mode),
    }
}

fn print_error(error: &NovaError) {
    match error {
        NovaError::Diagnostics(diagnostics) => {
            for diagnostic in diagnostics {
                eprint!(
                    "{}[{}]: {}",
                    diagnostic.severity.as_str(),
                    diagnostic.code,
                    diagnostic.message
                );
                if let Some(hint) = &diagnostic.hint {
                    eprint!("\n  help: {hint}");
                }
                eprintln!();
            }
        }
        error => eprintln!("error[{}]: {error}", error.code()),
    }
}

fn json_diagnostics(error: &NovaError) -> Vec<JsonValue> {
    match error {
        NovaError::Diagnostics(diagnostics) => diagnostics
            .iter()
            .map(|diagnostic| {
                json!({
                    "code": diagnostic.code,
                    "severity": diagnostic.severity.as_str(),
                    "message": diagnostic.message,
                    "hint": diagnostic.hint,
                    "path": "<repl>",
                    "line": null,
                    "column": null,
                })
            })
            .collect(),
        error => vec![json!({
            "code": error.code(),
            "severity": "error",
            "message": error.to_string(),
            "hint": null,
            "path": "<repl>",
            "line": null,
            "column": null,
        })],
    }
}
