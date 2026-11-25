use std::path::{Path, PathBuf};
use std::process::ExitCode;

use astra_compiler_api::{MigrateOptions, check_canonical, migrate_source};
use astra_core::edition::Edition;

use crate::{
    ReportedDiagnostic, byte_offset_to_line_col, discover_ns, display_path, edition_for_path,
    reported_json,
};

fn emit_reported(diagnostics: &[ReportedDiagnostic]) {
    crate::print_diagnostics(diagnostics);
}

fn targets(path: &str) -> Vec<String> {
    if Path::new(path).is_dir() {
        discover_ns(path)
    } else {
        vec![path.to_owned()]
    }
}

fn has(args: &[String], flag: &str) -> bool {
    args.iter().any(|arg| arg == flag)
}

fn positional(args: &[String]) -> String {
    args.iter()
        .find(|arg| !arg.starts_with('-'))
        .cloned()
        .unwrap_or_else(|| ".".to_owned())
}

fn json_diagnostic(
    diagnostic: &astra_core::diagnostic::Diagnostic,
    path: &str,
    text: &str,
) -> ReportedDiagnostic {
    let (line, column) = diagnostic
        .span
        .map(|span| byte_offset_to_line_col(text, span.start))
        .map(|(line, column)| (Some(line), Some(column)))
        .unwrap_or((None, None));
    reported_json(
        &diagnostic.code,
        "error",
        &diagnostic.message,
        diagnostic.hint.as_deref(),
        Some(path),
        line,
        column,
    )
}

pub fn cmd_canonical_check(args: &[String]) -> ExitCode {
    let json = has(args, "--json");
    let path = positional(args);
    let mut all = Vec::new();
    let mut has_errors = false;
    for file in targets(&path) {
        let text = match std::fs::read_to_string(&file) {
            Ok(text) => text,
            Err(error) => {
                eprintln!("astra-native: {}: {error}", display_path(&file));
                return ExitCode::FAILURE;
            }
        };
        let edition = match edition_for_path(&file) {
            Ok(edition) => edition,
            Err(error) => {
                eprintln!("astra-native: {}: {}", error.code, error.message);
                return ExitCode::FAILURE;
            }
        };
        let diagnostics = check_canonical(&text, edition);
        if !diagnostics.is_empty() {
            has_errors = true;
        }
        if json {
            for diagnostic in diagnostics {
                let reported = json_diagnostic(&diagnostic, &display_path(&file), &text);
                println!("{}", serde_json::to_string(&reported).unwrap_or_default());
            }
        } else {
            all.extend(
                diagnostics
                    .iter()
                    .map(|diagnostic| json_diagnostic(diagnostic, &display_path(&file), &text)),
            );
        }
    }

    if !json {
        emit_reported(&all);
    }
    if has_errors {
        ExitCode::FAILURE
    } else {
        ExitCode::SUCCESS
    }
}

fn nearest_manifest(file: &str) -> Option<PathBuf> {
    let mut dir = Path::new(file).parent().map(Path::to_path_buf)?;
    loop {
        for name in ["core.toml", "core.json"] {
            let candidate = dir.join(name);
            if candidate.exists() {
                return Some(candidate);
            }
        }
        if !dir.pop() {
            return None;
        }
    }
}

fn change_manifest_edition(file: &str) -> Result<bool, String> {
    let Some(manifest) = nearest_manifest(file) else {
        return Ok(false);
    };
    let text = std::fs::read_to_string(&manifest).map_err(|error| error.to_string())?;
    let updated = text
        .replace("edition = \"0.1\"", "edition = \"0.2\"")
        .replace("\"edition\": \"0.1\"", "\"edition\": \"0.2\"");
    if updated == text {
        return Ok(false);
    }
    std::fs::write(&manifest, updated).map_err(|error| error.to_string())?;
    Ok(true)
}

pub fn cmd_migrate(args: &[String]) -> ExitCode {
    let check = has(args, "--check");
    let dry_run = has(args, "--dry-run");
    let json = has(args, "--json");
    let change_edition = has(args, "--change-edition");
    let path = positional(args);
    let mut failures = 0usize;
    let mut changed = 0usize;

    for file in targets(&path) {
        let text = match std::fs::read_to_string(&file) {
            Ok(text) => text,
            Err(error) => {
                eprintln!("astra-native: {}: {error}", display_path(&file));
                failures += 1;
                continue;
            }
        };
        let options = MigrateOptions {
            target_edition: Edition::V0_2,
            change_edition,
        };
        let result = match migrate_source(&text, options) {
            Ok(result) => result,
            Err(diagnostics) => {
                let reported = diagnostics
                    .iter()
                    .map(|diagnostic| json_diagnostic(diagnostic, &display_path(&file), &text))
                    .collect::<Vec<_>>();
                emit_reported(&reported);
                failures += 1;
                continue;
            }
        };
        if result.changed {
            changed += 1;
        }
        if json {
            println!(
                "{}",
                serde_json::json!({
                    "path": display_path(&file),
                    "changed": result.changed,
                    "diagnostics": result.diagnostics,
                    "source": if dry_run { result.source.as_str() } else { "" },
                })
            );
        } else if dry_run {
            println!("{}", result.source);
        } else if !check
            && result.changed
            && let Err(error) = std::fs::write(&file, &result.source)
        {
            eprintln!("astra-native: {}: {error}", display_path(&file));
            failures += 1;
        }
        if check && result.changed {
            println!("would migrate {}", display_path(&file));
        }
        if change_edition {
            match change_manifest_edition(&file) {
                Ok(true) => println!("updated edition in nearest manifest"),
                Ok(false) => {}
                Err(error) => {
                    eprintln!("astra-native: manifest edition: {error}");
                    failures += 1;
                }
            }
        }
    }

    if failures > 0 || (check && changed > 0) {
        ExitCode::FAILURE
    } else {
        ExitCode::SUCCESS
    }
}
