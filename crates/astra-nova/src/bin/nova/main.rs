mod manifest;
mod repl;
mod test_runner;
mod watch;

use std::cell::RefCell;
use std::env;
use std::fs;
use std::process::ExitCode;
use std::rc::Rc;

use astra_core::diagnostic::Diagnostic;
use astra_core::mode::Mode;
use astra_core::parser::parse;
use astra_core::span::Span;
use astra_nova::{
    Host, NovaError, NovaOptions, ProjectError, ProjectOptions, Value as RuntimeValue,
    build_interpreter, build_project_interpreter, load_project, resolve_source_mode, run_source,
};
use serde_json::{Value as JsonValue, json};

const VERSION: &str = env!("CARGO_PKG_VERSION");

const HELP: &str = "\
Nova development runtime

Usage:
  nova <command> [options]

Commands:
  run [file.ns] [--watch] [--mode <mode>] [--json]
      Execute a source file, or the manifest [nova].entry when omitted.
  eval \"<source>\" [--mode <mode>] [--json]
      Evaluate a module, expression, or statement list directly.
  watch [file.ns] [--once] [--mode <mode>] [--json]
      Re-run a source file, or the manifest [nova].entry, when it changes.
  repl [--mode <mode>] [--json]
      Start an interactive session.
  test [path] [--watch] [--mode <mode>]
      Run @test functions in *.test.ns files. Falls back to main.
  version
      Print the Nova version.
  help
      Show this help.

Options:
  -h, --help              Show this help
  -v, --version           Show the Nova version
  --mode <mode>           core, app, web, or system (default: inferred)
  --json                  Print one JSON report (run, eval, watch, repl)

Exit codes:
  0  success
  1  source, input, test, or runtime failure
  2  usage error
";

enum Invocation {
    Run {
        file: Option<String>,
        mode: Mode,
        infer_mode: bool,
        json: bool,
        watch: bool,
    },
    Eval {
        source: String,
        mode: Mode,
        infer_mode: bool,
        json: bool,
    },
    Watch {
        file: Option<String>,
        mode: Mode,
        infer_mode: bool,
        json: bool,
        once: bool,
    },
    Repl {
        mode: Mode,
        infer_mode: bool,
        json: bool,
    },
    Test {
        path: String,
        mode: Mode,
        infer_mode: bool,
        watch: bool,
    },
    Help,
    Version,
}

struct ParsedArgs {
    mode: Mode,
    infer_mode: bool,
    json: bool,
    watch: bool,
    once: bool,
    positional: Vec<String>,
}

pub(crate) struct CliError {
    code: &'static str,
    message: String,
    command: Option<&'static str>,
    path: Option<String>,
    json: bool,
    exit_code: i32,
}

impl CliError {
    pub(crate) fn usage(
        message: impl Into<String>,
        command: Option<&'static str>,
        json: bool,
    ) -> Self {
        Self {
            code: "NS9000",
            message: message.into(),
            command,
            path: None,
            json,
            exit_code: 2,
        }
    }

    pub(crate) fn input(
        code: &'static str,
        message: impl Into<String>,
        command: &'static str,
        path: String,
        json: bool,
    ) -> Self {
        Self {
            code,
            message: message.into(),
            command: Some(command),
            path: Some(path),
            json,
            exit_code: 1,
        }
    }
}

fn main() -> ExitCode {
    let args: Vec<String> = env::args().skip(1).collect();
    ExitCode::from(run_cli(&args) as u8)
}

fn run_cli(args: &[String]) -> i32 {
    match parse_invocation(args) {
        Ok(Invocation::Help) => {
            print!("{HELP}");
            0
        }
        Ok(Invocation::Version) => {
            println!("nova {VERSION}");
            0
        }
        Ok(Invocation::Run {
            file,
            mode,
            infer_mode,
            json,
            watch,
        }) => {
            let file = match file {
                Some(file) => file,
                None => match manifest::resolve_entry("run", json) {
                    Ok(file) => file,
                    Err(error) => return emit_cli_error(&error),
                },
            };
            if watch {
                watch::watch_file(&file, mode, infer_mode, json, false)
            } else {
                execute_file(&file, mode, infer_mode, json)
            }
        }
        Ok(Invocation::Eval {
            source,
            mode,
            infer_mode,
            json,
        }) => execute(
            "eval",
            &prepare_eval_source(&source),
            "<eval>",
            mode,
            infer_mode,
            json,
            true,
        ),
        Ok(Invocation::Watch {
            file,
            mode,
            infer_mode,
            json,
            once,
        }) => {
            let file = match file {
                Some(file) => file,
                None => match manifest::resolve_entry("watch", json) {
                    Ok(file) => file,
                    Err(error) => return emit_cli_error(&error),
                },
            };
            watch::watch_file(&file, mode, infer_mode, json, once)
        }
        Ok(Invocation::Repl {
            mode,
            infer_mode,
            json,
        }) => repl::run(mode, infer_mode, json),
        Ok(Invocation::Test {
            path,
            mode,
            infer_mode,
            watch,
        }) => test_runner::run(&path, mode, infer_mode, watch),
        Err(error) => emit_cli_error(&error),
    }
}

fn parse_invocation(args: &[String]) -> Result<Invocation, CliError> {
    let json_requested = args.iter().any(|argument| argument == "--json");
    let Some(command) = args.first() else {
        return Ok(Invocation::Help);
    };

    if matches!(command.as_str(), "-h" | "--help" | "help") {
        return Ok(Invocation::Help);
    }
    if matches!(command.as_str(), "-v" | "--version" | "version") {
        return Ok(Invocation::Version);
    }
    if args
        .iter()
        .skip(1)
        .any(|argument| argument == "-h" || argument == "--help")
    {
        return Ok(Invocation::Help);
    }

    match command.as_str() {
        "run" => {
            let parsed = parse_args("run", &args[1..], true, true, false, json_requested)?;
            if parsed.positional.len() > 1 {
                return Err(CliError::usage(
                    "nova run accepts at most one source file.",
                    Some("run"),
                    json_requested,
                ));
            }
            Ok(Invocation::Run {
                file: parsed.positional.first().cloned(),
                mode: parsed.mode,
                infer_mode: parsed.infer_mode,
                json: parsed.json,
                watch: parsed.watch,
            })
        }
        "eval" => {
            let parsed = parse_args("eval", &args[1..], true, false, false, json_requested)?;
            let source = one_positional("eval", "source string", &parsed, json_requested)?;
            Ok(Invocation::Eval {
                source,
                mode: parsed.mode,
                infer_mode: parsed.infer_mode,
                json: parsed.json,
            })
        }
        "watch" => {
            let parsed = parse_args("watch", &args[1..], true, false, true, json_requested)?;
            if parsed.positional.len() > 1 {
                return Err(CliError::usage(
                    "nova watch accepts at most one source file.",
                    Some("watch"),
                    json_requested,
                ));
            }
            Ok(Invocation::Watch {
                file: parsed.positional.first().cloned(),
                mode: parsed.mode,
                infer_mode: parsed.infer_mode,
                json: parsed.json,
                once: parsed.once,
            })
        }
        "repl" => {
            let parsed = parse_args("repl", &args[1..], true, false, false, json_requested)?;
            if !parsed.positional.is_empty() {
                return Err(CliError::usage(
                    "nova repl does not accept positional arguments.",
                    Some("repl"),
                    json_requested,
                ));
            }
            Ok(Invocation::Repl {
                mode: parsed.mode,
                infer_mode: parsed.infer_mode,
                json: parsed.json,
            })
        }
        "test" => {
            let parsed = parse_args("test", &args[1..], false, true, false, json_requested)?;
            if parsed.positional.len() > 1 {
                return Err(CliError::usage(
                    "nova test accepts at most one file or directory.",
                    Some("test"),
                    json_requested,
                ));
            }
            Ok(Invocation::Test {
                path: parsed
                    .positional
                    .first()
                    .cloned()
                    .unwrap_or_else(|| ".".to_owned()),
                mode: parsed.mode,
                infer_mode: parsed.infer_mode,
                watch: parsed.watch,
            })
        }
        other => Err(CliError::usage(
            format!("Unknown command '{other}'. Use 'nova help'."),
            None,
            json_requested,
        )),
    }
}

fn parse_args(
    command: &'static str,
    args: &[String],
    allow_json: bool,
    allow_watch: bool,
    allow_once: bool,
    json_requested: bool,
) -> Result<ParsedArgs, CliError> {
    let mut parsed = ParsedArgs {
        mode: Mode::Core,
        infer_mode: true,
        json: false,
        watch: false,
        once: false,
        positional: Vec::new(),
    };
    let mut index = 0;

    while index < args.len() {
        let argument = &args[index];

        if argument == "--" {
            parsed.positional.extend(args[index + 1..].iter().cloned());
            break;
        }

        match argument.as_str() {
            "--json" if allow_json => {
                parsed.json = true;
                index += 1;
            }
            "--watch" if allow_watch => {
                parsed.watch = true;
                index += 1;
            }
            "--once" if allow_once => {
                parsed.once = true;
                index += 1;
            }
            "--mode" => {
                let Some(value) = args.get(index + 1) else {
                    return Err(CliError::usage(
                        "The --mode option requires core, app, web, or system.",
                        Some(command),
                        json_requested,
                    ));
                };
                let Some(mode) = Mode::parse(value) else {
                    return Err(CliError::usage(
                        format!("Unknown mode '{value}'. Expected core, app, web, or system."),
                        Some(command),
                        json_requested,
                    ));
                };
                parsed.mode = mode;
                parsed.infer_mode = false;
                index += 2;
            }
            value if value.starts_with('-') => {
                return Err(CliError::usage(
                    format!("Unknown {command} option '{value}'."),
                    Some(command),
                    json_requested,
                ));
            }
            value => {
                parsed.positional.push(value.to_owned());
                index += 1;
            }
        }
    }

    Ok(parsed)
}

fn one_positional(
    command: &'static str,
    description: &str,
    parsed: &ParsedArgs,
    json_requested: bool,
) -> Result<String, CliError> {
    if parsed.positional.len() != 1 {
        return Err(CliError::usage(
            format!("nova {command} expects exactly one {description}."),
            Some(command),
            json_requested,
        ));
    }

    Ok(parsed.positional[0].clone())
}

fn project_root(file: &std::path::Path) -> std::path::PathBuf {
    let entry = fs::canonicalize(file).unwrap_or_else(|_| file.to_path_buf());
    if let Ok(current) = env::current_dir() {
        let current = fs::canonicalize(&current).unwrap_or(current);
        if entry.starts_with(&current) {
            return current;
        }
    }
    file.parent()
        .unwrap_or_else(|| std::path::Path::new("."))
        .to_path_buf()
}

pub(crate) fn execute_file(path: &str, mode: Mode, infer_mode: bool, json: bool) -> i32 {
    let file = std::path::Path::new(path);
    let root = project_root(file);
    let project = match load_project(ProjectOptions {
        root: root.clone(),
        entry: file.to_path_buf(),
        mode,
        infer_mode,
        allow_root_escape: false,
    }) {
        Ok(project) => project,
        Err(error) => return emit_project_error(&error, "run", mode, json),
    };
    let selected_mode = project.mode;
    let entry_id = project.entry.clone();
    let entry_path = project
        .modules
        .get(&entry_id)
        .map_or_else(|| file.to_path_buf(), |module| module.path.clone());
    let options = NovaOptions {
        mode: selected_mode,
        infer_mode: false,
        ..NovaOptions::default()
    };

    let stdout = Rc::new(RefCell::new(String::new()));
    let stderr = Rc::new(RefCell::new(String::new()));
    let host = CaptureHost {
        stdout: stdout.clone(),
        stderr: stderr.clone(),
    };
    let result = match build_project_interpreter(project, options, Box::new(host)) {
        Ok(mut interpreter) => interpreter.run(),
        Err(error) => {
            let stdout = stdout.borrow().clone();
            let stderr = stderr.borrow().clone();
            return emit_project_error_with_output(
                &error,
                "run",
                selected_mode,
                json,
                &stdout,
                &stderr,
            );
        }
    };
    let stdout = stdout.borrow().clone();
    let stderr = stderr.borrow().clone();

    if json {
        let (ok, diagnostics) = match result {
            Ok(_) => (true, Vec::new()),
            Err(error) => {
                let source = fs::read_to_string(&entry_path).unwrap_or_default();
                (
                    false,
                    json_diagnostics(&error, &entry_path.display().to_string(), &source),
                )
            }
        };
        emit_json_report(
            ok,
            Some("run"),
            selected_mode,
            None,
            &stdout,
            &stderr,
            diagnostics,
        );
        return if ok { 0 } else { 1 };
    }

    print!("{stdout}");
    eprint!("{stderr}");
    match result {
        Ok(_) => 0,
        Err(error) => {
            let source = fs::read_to_string(&entry_path).unwrap_or_default();
            print_error_text(&error, &entry_path.display().to_string(), &source);
            1
        }
    }
}

fn emit_project_error_with_output(
    error: &ProjectError,
    command: &'static str,
    mode: Mode,
    json: bool,
    stdout: &str,
    stderr: &str,
) -> i32 {
    if !stdout.is_empty() {
        print!("{stdout}");
    }
    if !stderr.is_empty() {
        eprint!("{stderr}");
    }
    emit_project_error(error, command, mode, json)
}

fn emit_project_error(error: &ProjectError, command: &'static str, mode: Mode, json: bool) -> i32 {
    let diagnostics = project_error_json(error);
    if json {
        emit_json_report(false, Some(command), mode, None, "", "", diagnostics);
    } else {
        match error {
            ProjectError::Diagnostics(diagnostics) => {
                for diagnostic in diagnostics {
                    let source = fs::read_to_string(&diagnostic.path).unwrap_or_default();
                    print_diagnostic_text(
                        &diagnostic.diagnostic,
                        &diagnostic.path.display().to_string(),
                        &source,
                    );
                }
            }
            ProjectError::Runtime { path, error, .. } => {
                let source = fs::read_to_string(path).unwrap_or_default();
                print_error_text(error, &path.display().to_string(), &source);
            }
            ProjectError::InvalidModule { id, message } => {
                eprintln!("NS3012: invalid module id '{id}': {message}");
            }
        }
    }
    1
}

fn project_error_json(error: &ProjectError) -> Vec<JsonValue> {
    match error {
        ProjectError::Diagnostics(diagnostics) => diagnostics
            .iter()
            .map(|diagnostic| {
                let source = fs::read_to_string(&diagnostic.path).unwrap_or_default();
                frontend_json(
                    &diagnostic.diagnostic,
                    &diagnostic.path.display().to_string(),
                    &source,
                )
            })
            .collect(),
        ProjectError::Runtime { path, error, .. } => {
            let source = fs::read_to_string(path).unwrap_or_default();
            vec![runtime_json(error, &path.display().to_string(), &source)]
        }
        ProjectError::InvalidModule { id, message } => vec![json!({
            "code": "NS3012",
            "severity": "error",
            "message": message,
            "hint": null,
            "path": id,
            "line": null,
            "column": null,
        })],
    }
}

fn prepare_eval_source(source: &str) -> String {
    if parse(source).is_ok() {
        return source.to_owned();
    }

    let wrapped = format!("fn main() {{\n{source}\n}}\n");
    if parse(&wrapped).is_ok() {
        wrapped
    } else {
        source.to_owned()
    }
}

fn execute(
    command: &'static str,
    source: &str,
    path: &str,
    mode: Mode,
    infer_mode: bool,
    json: bool,
    print_value: bool,
) -> i32 {
    let options = NovaOptions {
        mode,
        infer_mode,
        ..NovaOptions::default()
    };
    let mode = match resolve_source_mode(source, options) {
        Ok(mode) => mode,
        Err(error) => {
            if json {
                emit_json_report(
                    false,
                    Some(command),
                    mode,
                    None,
                    "",
                    "",
                    json_diagnostics(&error, path, source),
                );
            } else {
                print_error_text(&error, path, source);
            }
            return 1;
        }
    };
    let options = NovaOptions {
        mode,
        infer_mode: false,
        ..options
    };

    if json {
        let (result, stdout, stderr) = captured_call(source, options, None);
        let (ok, value, diagnostics) = match result {
            Ok(value) => (
                true,
                print_value.then(|| rendered_value(&value)).flatten(),
                Vec::new(),
            ),
            Err(error) => (false, None, json_diagnostics(&error, path, source)),
        };

        emit_json_report(
            ok,
            Some(command),
            mode,
            value.as_deref(),
            &stdout,
            &stderr,
            diagnostics,
        );
        return if ok { 0 } else { 1 };
    }

    match run_source(source, options) {
        Ok(value) => {
            if print_value && let Some(value) = rendered_value(&value) {
                println!("{value}");
            }
            0
        }
        Err(error) => {
            print_error_text(&error, path, source);
            1
        }
    }
}

pub(crate) fn rendered_value(value: &RuntimeValue) -> Option<String> {
    match value {
        RuntimeValue::Unit => None,
        value => Some(value.render()),
    }
}

pub(crate) fn captured_call(
    source: &str,
    options: NovaOptions,
    function: Option<&str>,
) -> (Result<RuntimeValue, NovaError>, String, String) {
    let stdout = Rc::new(RefCell::new(String::new()));
    let stderr = Rc::new(RefCell::new(String::new()));
    let host = CaptureHost {
        stdout: stdout.clone(),
        stderr: stderr.clone(),
    };

    let result = match build_interpreter(source, options, Box::new(host)) {
        Ok(mut interpreter) => match function {
            Some(name) => interpreter.call(name, Vec::new()),
            None => interpreter.run(),
        },
        Err(error) => Err(error),
    };

    let stdout = stdout.borrow().clone();
    let stderr = stderr.borrow().clone();
    (result, stdout, stderr)
}

struct CaptureHost {
    stdout: Rc<RefCell<String>>,
    stderr: Rc<RefCell<String>>,
}

impl Host for CaptureHost {
    fn stdout(&mut self, text: &str) {
        self.stdout.borrow_mut().push_str(text);
    }

    fn stderr(&mut self, text: &str) {
        self.stderr.borrow_mut().push_str(text);
    }
}

pub(crate) fn emit_cli_error(error: &CliError) -> i32 {
    if error.json {
        emit_json_report(
            false,
            error.command,
            Mode::Core,
            None,
            "",
            "",
            vec![json!({
                "code": error.code,
                "severity": "error",
                "message": error.message,
                "hint": null,
                "path": error.path,
                "line": null,
                "column": null,
            })],
        );
    } else {
        eprintln!("{}: {}", error.code, error.message);
    }

    error.exit_code
}

pub(crate) fn emit_json_report(
    ok: bool,
    command: Option<&str>,
    mode: Mode,
    value: Option<&str>,
    stdout: &str,
    stderr: &str,
    diagnostics: Vec<JsonValue>,
) {
    let report = json!({
        "ok": ok,
        "command": command,
        "mode": mode.as_str(),
        "value": value,
        "stdout": stdout,
        "stderr": stderr,
        "diagnostics": diagnostics,
    });

    println!(
        "{}",
        serde_json::to_string(&report).expect("JSON serialization cannot fail")
    );
}

pub(crate) fn json_diagnostics(error: &NovaError, path: &str, source: &str) -> Vec<JsonValue> {
    match error {
        NovaError::Diagnostics(diagnostics) => diagnostics
            .iter()
            .map(|diagnostic| frontend_json(diagnostic, path, source))
            .collect(),
        error => vec![runtime_json(error, path, source)],
    }
}

fn frontend_json(diagnostic: &Diagnostic, path: &str, source: &str) -> JsonValue {
    let (line, column) = position(source, diagnostic.span);
    json!({
        "code": diagnostic.code,
        "severity": diagnostic.severity.as_str(),
        "message": diagnostic.message,
        "hint": diagnostic.hint,
        "path": path,
        "line": line,
        "column": column,
    })
}

fn runtime_json(error: &NovaError, path: &str, source: &str) -> JsonValue {
    let (line, column) = position(source, error_span(error));
    json!({
        "code": error.code(),
        "severity": "error",
        "message": error.to_string(),
        "hint": null,
        "path": path,
        "line": line,
        "column": column,
    })
}

pub(crate) fn print_error_text(error: &NovaError, path: &str, source: &str) {
    match error {
        NovaError::Diagnostics(diagnostics) => {
            for diagnostic in diagnostics {
                print_diagnostic_text(diagnostic, path, source);
            }
        }
        error => {
            let location = format_location(path, source, error_span(error));
            eprintln!("{location}error[{}]: {error}", error.code());
        }
    }
}

fn print_diagnostic_text(diagnostic: &Diagnostic, path: &str, source: &str) {
    let location = format_location(path, source, diagnostic.span);
    eprintln!(
        "{location}{}[{}]: {}",
        diagnostic.severity.as_str(),
        diagnostic.code,
        diagnostic.message
    );
    if let Some(hint) = &diagnostic.hint {
        eprintln!("  help: {hint}");
    }
}

fn format_location(path: &str, source: &str, span: Option<Span>) -> String {
    match position(source, span) {
        (Some(line), Some(column)) => format!("{path}:{line}:{column}: "),
        _ => format!("{path}: "),
    }
}

fn position(source: &str, span: Option<Span>) -> (Option<usize>, Option<usize>) {
    let Some(span) = span else {
        return (None, None);
    };

    let mut end = span.start.min(source.len());
    while end > 0 && !source.is_char_boundary(end) {
        end -= 1;
    }

    let prefix = &source[..end];
    let line = prefix.bytes().filter(|byte| *byte == b'\n').count() + 1;
    let column = prefix.rsplit('\n').next().unwrap_or("").chars().count() + 1;
    (Some(line), Some(column))
}

pub(crate) fn error_span(error: &NovaError) -> Option<Span> {
    match error {
        NovaError::ManualMemoryUnsupported { span } => Some(*span),
        NovaError::ForeignScriptUnsupported { span, .. } => Some(*span),
        NovaError::Unsupported { span, .. } => *span,
        NovaError::Panic(panic) => panic.span,
        NovaError::Abort(abort) => abort.span,
        NovaError::Diagnostics(_)
        | NovaError::MissingEntry { .. }
        | NovaError::UnhandledFailure(_)
        | NovaError::Internal(_) => None,
    }
}
