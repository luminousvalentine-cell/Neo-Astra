use std::env;
use std::fs;
use std::process::ExitCode;

use astra_capabilities::infer_module_mode_named;
use astra_compiler_api::{
    API_VERSION, AstResponse, CompileRequest, CompilerApi, ExplainResponse, FormatResponse,
    FrontendCompiler, SourceUnit,
};
use astra_core::formatter::format_source;
use astra_core::mode::Mode;
use astra_core::parser::parse;

fn main() -> ExitCode {
    let args: Vec<String> = env::args().collect();
    let args = &args[1..];
    if args.is_empty() {
        print_help();
        return ExitCode::SUCCESS;
    }
    match args[0].as_str() {
        "--version" | "-V" => {
            println!("astra {}", env!("CARGO_PKG_VERSION"));
            ExitCode::SUCCESS
        }
        "--help" | "-h" => {
            print_help();
            ExitCode::SUCCESS
        }
        "check" => cmd_check(&args[1..]),
        "fmt" => cmd_fmt(&args[1..]),
        "ast" => cmd_ast(&args[1..]),
        "explain" => cmd_explain(&args[1..]),
        other => {
            eprintln!("astra-native: unknown command '{other}'");
            ExitCode::FAILURE
        }
    }
}

fn print_help() {
    println!("astra-native - native CLI preview");
    println!();
    println!("Usage: astra-native <command> [options]");
    println!();
    println!("Commands:");
    println!("  check <file.ns> [--json] [--mode <mode>] [--target <name>]");
    println!("  fmt <file.ns> [--check] [--json]");
    println!("  ast <file.ns> [--json]");
    println!("  explain <file.ns> [--json] [--mode <mode>] [--target <name>]");
    println!();
    println!("Modes: core, app, web, system");
    println!("Targets: js, ts, c");
}

fn read_file(path: &str) -> Result<String, ExitCode> {
    fs::read_to_string(path).map_err(|error| {
        eprintln!("astra-native: cannot read '{path}': {error}");
        ExitCode::FAILURE
    })
}

fn parse_mode(value: &str) -> Result<Mode, ExitCode> {
    Mode::parse(value).ok_or_else(|| {
        eprintln!("astra-native: unsupported mode '{value}'");
        ExitCode::FAILURE
    })
}

fn parse_target(value: &str) -> Result<String, ExitCode> {
    if matches!(value, "js" | "ts" | "c") {
        Ok(value.to_owned())
    } else {
        eprintln!("astra-native: unsupported target '{value}'");
        Err(ExitCode::FAILURE)
    }
}

fn fail_unknown(flag: &str) -> ExitCode {
    eprintln!("astra-native: unknown flag '{flag}'");
    ExitCode::FAILURE
}

fn cmd_check(args: &[String]) -> ExitCode {
    let mut file: Option<String> = None;
    let mut json = false;
    let mut mode: Option<Mode> = None;
    let mut target = "js".to_owned();
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--json" => {
                json = true;
                i += 1;
            }
            "--mode" => {
                if i + 1 >= args.len() {
                    eprintln!("astra-native: --mode requires a value");
                    return ExitCode::FAILURE;
                }
                match parse_mode(&args[i + 1]) {
                    Ok(m) => mode = Some(m),
                    Err(code) => return code,
                }
                i += 2;
            }
            "--target" => {
                if i + 1 >= args.len() {
                    eprintln!("astra-native: --target requires a value");
                    return ExitCode::FAILURE;
                }
                match parse_target(&args[i + 1]) {
                    Ok(t) => target = t,
                    Err(code) => return code,
                }
                i += 2;
            }
            other if !other.starts_with('-') && file.is_none() => {
                file = Some(other.to_owned());
                i += 1;
            }
            other => return fail_unknown(other),
        }
    }
    let Some(file) = file else {
        eprintln!("astra-native: missing file argument");
        return ExitCode::FAILURE;
    };
    let text = match read_file(&file) {
        Ok(t) => t,
        Err(code) => return code,
    };

    let request = CompileRequest {
        api_version: API_VERSION.to_owned(),
        source: SourceUnit {
            path: file.clone(),
            text,
        },
        target,
        mode,
        integer_policy: Default::default(),
    };
    let compiler = FrontendCompiler;
    let response = compiler.check(&request);

    if json {
        println!("{}", serde_json::to_string(&response).unwrap_or_default());
    } else {
        for diagnostic in &response.diagnostics {
            eprintln!("{diagnostic}");
        }
    }

    if response.program.is_some() && response.diagnostics.is_empty() {
        ExitCode::SUCCESS
    } else {
        ExitCode::FAILURE
    }
}

fn cmd_fmt(args: &[String]) -> ExitCode {
    let mut file: Option<String> = None;
    let mut check = false;
    let mut json = false;
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--check" => {
                check = true;
                i += 1;
            }
            "--json" => {
                json = true;
                i += 1;
            }
            other if !other.starts_with('-') && file.is_none() => {
                file = Some(other.to_owned());
                i += 1;
            }
            other => return fail_unknown(other),
        }
    }
    let Some(file) = file else {
        eprintln!("astra-native: missing file argument");
        return ExitCode::FAILURE;
    };
    let text = match read_file(&file) {
        Ok(t) => t,
        Err(code) => return code,
    };

    match format_source(&text) {
        Ok(formatted) => {
            let changed = formatted != text;
            if json {
                println!(
                    "{}",
                    serde_json::to_string(&FormatResponse::success(formatted.clone()))
                        .unwrap_or_default()
                );
            }
            if check {
                if changed {
                    ExitCode::FAILURE
                } else {
                    ExitCode::SUCCESS
                }
            } else {
                if let Err(error) = fs::write(&file, formatted) {
                    eprintln!("astra-native: cannot write '{file}': {error}");
                    return ExitCode::FAILURE;
                }
                ExitCode::SUCCESS
            }
        }
        Err(diagnostics) => {
            for diagnostic in &diagnostics {
                eprintln!("{diagnostic}");
            }
            if json {
                println!(
                    "{}",
                    serde_json::to_string(&FormatResponse::failure(diagnostics))
                        .unwrap_or_default()
                );
            }
            ExitCode::FAILURE
        }
    }
}

fn cmd_ast(args: &[String]) -> ExitCode {
    let mut file: Option<String> = None;
    let mut json = false;
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--json" => {
                json = true;
                i += 1;
            }
            other if !other.starts_with('-') && file.is_none() => {
                file = Some(other.to_owned());
                i += 1;
            }
            other => return fail_unknown(other),
        }
    }
    let Some(file) = file else {
        eprintln!("astra-native: missing file argument");
        return ExitCode::FAILURE;
    };
    let text = match read_file(&file) {
        Ok(t) => t,
        Err(code) => return code,
    };

    match parse(&text) {
        Ok(module) => {
            if json {
                println!(
                    "{}",
                    serde_json::to_string(&AstResponse::success(module)).unwrap_or_default()
                );
            } else {
                println!("{module:?}");
            }
            ExitCode::SUCCESS
        }
        Err(diagnostics) => {
            for diagnostic in &diagnostics {
                eprintln!("{diagnostic}");
            }
            if json {
                println!(
                    "{}",
                    serde_json::to_string(&AstResponse::failure(diagnostics)).unwrap_or_default()
                );
            }
            ExitCode::FAILURE
        }
    }
}

fn cmd_explain(args: &[String]) -> ExitCode {
    let mut file: Option<String> = None;
    let mut json = false;
    let mut mode: Option<Mode> = None;
    let mut target = "js".to_owned();
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--json" => {
                json = true;
                i += 1;
            }
            "--mode" => {
                if i + 1 >= args.len() {
                    eprintln!("astra-native: --mode requires a value");
                    return ExitCode::FAILURE;
                }
                match parse_mode(&args[i + 1]) {
                    Ok(m) => mode = Some(m),
                    Err(code) => return code,
                }
                i += 2;
            }
            "--target" => {
                if i + 1 >= args.len() {
                    eprintln!("astra-native: --target requires a value");
                    return ExitCode::FAILURE;
                }
                match parse_target(&args[i + 1]) {
                    Ok(t) => target = t,
                    Err(code) => return code,
                }
                i += 2;
            }
            other if !other.starts_with('-') && file.is_none() => {
                file = Some(other.to_owned());
                i += 1;
            }
            other => return fail_unknown(other),
        }
    }
    let _ = (mode, target);
    let Some(file) = file else {
        eprintln!("astra-native: missing file argument");
        return ExitCode::FAILURE;
    };
    let text = match read_file(&file) {
        Ok(t) => t,
        Err(code) => return code,
    };

    match parse(&text) {
        Ok(module) => {
            let inference = infer_module_mode_named(&module, file.clone());
            if json {
                println!(
                    "{}",
                    serde_json::to_string(&ExplainResponse::success(inference.clone()))
                        .unwrap_or_default()
                );
            } else {
                println!("{}", inference.explain());
            }
            if inference.is_ok() {
                ExitCode::SUCCESS
            } else {
                ExitCode::FAILURE
            }
        }
        Err(diagnostics) => {
            for diagnostic in &diagnostics {
                eprintln!("{diagnostic}");
            }
            if json {
                println!(
                    "{}",
                    serde_json::to_string(&ExplainResponse::failure(diagnostics))
                        .unwrap_or_default()
                );
            }
            ExitCode::FAILURE
        }
    }
}
