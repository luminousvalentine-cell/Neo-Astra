use std::env;
use std::fs;
use std::process::ExitCode;

use astra_capabilities::infer_module_mode_named;
use astra_compiler_api::{
    API_VERSION, CompileRequest, CompilerApi, FrontendCompiler, IntegerPolicy, ProjectCheckRequest,
    ProjectSourceUnit, SourceUnit,
};
use astra_core::ast::{Item, ItemKind};
use astra_core::diagnostic::Diagnostic;
use astra_core::edition::Edition;
use astra_core::formatter::{FormatOptions, format_source_with_options};
use astra_core::mode::Mode;
use astra_core::parser::{ParseOptions, parse_with_options};
use astra_manifest::{
    BuildManifest, CoreManifest, format_build_json, format_build_toml, format_core_json,
    format_core_toml, parse_build_json, parse_build_toml, parse_core_json, parse_core_toml,
};
use astra_toolchain::doctor::doctor_with_download as run_doctor;
use astra_toolchain::policy::parse_toolchain_policy;

mod build;
mod error_catalog;
mod migrate_cmd;
mod render;

const VERSION: &str = env!("CARGO_PKG_VERSION");

#[derive(serde::Serialize)]
struct ManifestInfo {
    path: String,
    name: String,
    format: String,
}

#[derive(serde::Serialize, serde::Deserialize)]
struct ReportedRelated {
    file: String,
    start: usize,
    end: usize,
    message: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    line: Option<usize>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    column: Option<usize>,
}

#[derive(serde::Serialize, serde::Deserialize)]
struct ReportedDiagnostic {
    code: String,
    severity: String,
    message: String,
    hint: Option<String>,
    path: Option<String>,
    line: Option<usize>,
    column: Option<usize>,

    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    related: Vec<ReportedRelated>,

    #[serde(skip)]
    span: Option<(usize, usize)>,

    #[serde(skip)]
    snippet: Option<String>,
}

impl ReportedDiagnostic {
    pub(crate) fn with_span(mut self, span: Option<(usize, usize)>) -> Self {
        self.span = span;
        self
    }

    pub(crate) fn with_related(mut self, related: Vec<ReportedRelated>) -> Self {
        self.related = related;
        self
    }

    pub(crate) fn with_snippet(mut self, text: &str) -> Self {
        let Some((start, end)) = self.span else {
            return self;
        };
        if start > text.len() {
            return self;
        }
        let line = match self.line {
            Some(line) => line,
            None => {
                let (line, column) = byte_offset_to_line_col(text, start);
                self.line = Some(line);
                self.column = Some(column);
                line
            }
        };
        self.snippet = render::snippet(text, start, end, line);
        self
    }
}

#[derive(serde::Serialize)]
struct ManifestDiagnostic {
    code: &'static str,
    severity: &'static str,
    message: String,
    path: String,
}

#[derive(serde::Serialize)]
struct CheckReport {
    manifest: ManifestInfo,
    package: Option<String>,
    ok: bool,
    diagnostics: Vec<ReportedDiagnostic>,
}

struct ManifestError {
    code: &'static str,
    message: String,
    path: String,
}

struct ProjectManifests {
    root: std::path::PathBuf,
    core_path: String,
    core_name: String,
    core_format: String,
    core: CoreManifest,
    build: BuildManifest,
    build_present: bool,
}

impl ProjectManifests {
    fn entry_value(&self) -> String {
        self.build
            .nova
            .as_ref()
            .and_then(|nova| nova.entry.clone())
            .filter(|entry| !entry.is_empty())
            .unwrap_or_else(|| "src/main.ns".to_owned())
    }

    fn build_default_target(&self) -> Option<String> {
        self.build
            .build
            .as_ref()
            .and_then(|build| build.default_target.clone())
    }

    fn active_target(&self) -> String {
        self.build_default_target()
            .unwrap_or_else(|| "native".to_owned())
    }

    fn default_mode(&self) -> Option<Mode> {
        self.build
            .build
            .as_ref()
            .and_then(|build| build.default_mode.as_deref())
            .and_then(Mode::parse)
    }

    fn source_check_target(&self) -> String {
        let target = if self.build_present {
            self.build_default_target()
                .unwrap_or_else(|| "js".to_owned())
        } else {
            "native".to_owned()
        };
        match target.as_str() {
            "native" | "c" => "c".to_owned(),
            "ts" => "ts".to_owned(),
            _ => "js".to_owned(),
        }
    }

    fn integer_policy(&self) -> IntegerPolicy {
        match self
            .build
            .math
            .as_ref()
            .and_then(|math| math.get("integers"))
            .and_then(|value| value.as_str())
        {
            Some("safe-checked") => IntegerPolicy::SafeChecked,
            Some("arbitrary") => IntegerPolicy::Arbitrary,
            _ => IntegerPolicy::Wrapping,
        }
    }

    fn edition(&self) -> Edition {
        self.core
            .package
            .as_ref()
            .and_then(|package| package.edition.as_deref())
            .and_then(|edition| edition.parse::<Edition>().ok())
            .unwrap_or_default()
    }

    fn package_name(&self) -> Option<String> {
        self.core
            .package
            .as_ref()
            .and_then(|package| package.name.clone())
    }
}

#[derive(serde::Serialize)]
struct FmtFileReport {
    path: String,
    status: String,
    diagnostics: Vec<serde_json::Value>,
}

#[derive(serde::Serialize)]
struct FmtReport {
    check: bool,
    ok: bool,
    files: Vec<FmtFileReport>,
    diagnostics: Vec<serde_json::Value>,
}

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct NativeAstResponse {
    api_version: String,
    ok: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    ast: Option<serde_json::Value>,
    diagnostics: Vec<serde_json::Value>,
}

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct NativeExplainResponse {
    api_version: String,
    ok: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    inference: Option<serde_json::Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    explanation: Option<String>,
    diagnostics: Vec<serde_json::Value>,
}

fn main() -> ExitCode {
    let args: Vec<String> = env::args().collect();
    let args = &args[1..];
    if args.is_empty() {
        print_help();
        return ExitCode::SUCCESS;
    }
    match args[0].as_str() {
        "--version" | "-v" | "-V" | "version" => {
            println!("astra {VERSION}");
            ExitCode::SUCCESS
        }
        "--help" | "-h" | "help" => {
            print_help();
            ExitCode::SUCCESS
        }
        "check" => {
            if args[1..].iter().any(|arg| arg == "--canonical-syntax") {
                migrate_cmd::cmd_canonical_check(&args[1..])
            } else {
                cmd_check(&args[1..])
            }
        }
        "nopm" => {
            let code = astra_nopm::run(&args[1..]);
            std::process::exit(code);
        }
        "doctor" => cmd_doctor(&args[1..]),
        "build" => build::cmd_build(&args[1..]),
        "fmt" => {
            if args[1..].iter().any(|arg| arg == "--canonical") {
                migrate_cmd::cmd_canonical_check(&args[1..])
            } else {
                cmd_fmt(&args[1..])
            }
        }
        "migrate" => migrate_cmd::cmd_migrate(&args[1..]),
        "ast" => cmd_ast(&args[1..]),
        "explain" => cmd_explain(&args[1..]),
        other => {
            eprintln!("astra-native: unknown command '{other}'");
            ExitCode::FAILURE
        }
    }
}

fn discover_ns(dir: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut stack = vec![std::path::PathBuf::from(dir)];
    while let Some(d) = stack.pop() {
        if let Ok(entries) = fs::read_dir(&d) {
            for e in entries.flatten() {
                let p = e.path();
                let name = e.file_name().to_string_lossy().to_string();
                if p.is_dir() {
                    if matches!(
                        name.as_str(),
                        "build"
                            | "coverage"
                            | "dist"
                            | "node_modules"
                            | "out"
                            | "target"
                            | ".astra"
                            | ".turbo"
                            | ".next"
                    ) {
                        continue;
                    }
                    if name.starts_with('.') {
                        continue;
                    }
                    stack.push(p);
                } else if p.extension().map(|x| x == "ns").unwrap_or(false) {
                    if name.starts_with('.') {
                        continue;
                    }
                    out.push(p.to_string_lossy().to_string());
                }
            }
        }
    }
    out.sort();
    out
}

fn scan_import_specifiers(source: &str) -> Vec<String> {
    use std::collections::BTreeSet;
    let mut specifiers = BTreeSet::new();

    for line in source.lines() {
        let trimmed = line.trim_start();

        if let Some(idx) = trimmed.find("from") {
            let rest = &trimmed[idx..];
            if let Some(q1) = rest.find('"')
                && let Some(q2) = rest[q1 + 1..].find('"')
            {
                let spec = &rest[q1 + 1..q1 + 1 + q2];

                if trimmed.starts_with("import") || trimmed.starts_with("export") {
                    specifiers.insert(spec.to_owned());
                }
            }
        }

        if trimmed.starts_with("import")
            && !trimmed.contains(" from ")
            && let Some(q1) = trimmed.find('"')
            && let Some(q2) = trimmed[q1 + 1..].find('"')
        {
            let spec = &trimmed[q1 + 1..q1 + 1 + q2];
            specifiers.insert(spec.to_owned());
        }
    }

    for line in source.lines() {
        let trimmed = line.trim();
        if let Some(rest) = trimmed.strip_prefix("import ") {
            let rest = rest.trim_end_matches(';').trim();
            if !rest.contains('"')
                && !rest.contains(" from ")
                && rest
                    .chars()
                    .all(|c| c.is_alphanumeric() || c == '.' || c == '_')
                && rest.contains('.')
            {
                specifiers.insert(rest.to_owned());
            }
        }
    }
    specifiers.into_iter().collect()
}

fn resolve_project_specifier(
    importer_key: &str,
    specifier: &str,
    root: &std::path::Path,
) -> Option<String> {
    if specifier.starts_with("./") || specifier.starts_with("../") {
        let importer_dir = std::path::Path::new(importer_key)
            .parent()
            .unwrap_or(std::path::Path::new(""));
        let target = importer_dir.join(specifier);
        let mut target = target.to_string_lossy().replace('\\', "/");
        if !target.to_lowercase().ends_with(".ns") {
            target = format!("{}.ns", target);
        }

        let mut parts: Vec<&str> = Vec::new();
        for seg in target.split('/') {
            if seg.is_empty() || seg == "." {
                continue;
            }
            if seg == ".." {
                parts.pop();
            } else {
                parts.push(seg);
            }
        }
        let normalized = parts.join("/");

        if normalized.is_empty() || normalized.starts_with("../") || normalized == ".." {
            return None;
        }
        let full = root.join(&normalized);
        if !full.exists() {
            return None;
        }
        Some(normalized)
    } else if !specifier.contains('/')
        && specifier.contains('.')
        && !specifier.to_lowercase().ends_with(".ns")
    {
        Some(format!("{}.ns", specifier.replace('.', "/")))
    } else {
        None
    }
}

fn print_help() {
    println!("Neo Astra compiler");
    println!();
    println!("Usage:");
    println!("  astra <command> [options]");
    println!();
    println!("Commands:");
    println!("  check [dir] [--json]   Discover, validate, and check the project");
    println!("  doctor [dir] [--json]  Resolve and print the toolchain ladder");
    println!("  build [dir]            Build the configured target");
    println!("  fmt [path] [--check]   Format .ns source and core.json/core.toml");
    println!("  migrate [path]         Rewrite v0.1 source to canonical edition 0.2");
    println!("                           (--check, --dry-run, --json, --change-edition)");
    println!("  ast [path]             Print the parsed AST of a file or the project entry");
    println!("  explain [path]         Explain mode/capability inference for a file or entry");
    println!();
    println!("Environment:");
    println!("  ASTRA_CC               Override the C compiler (path or command)");
    println!();
    println!("Options:");
    println!("  -h, --help             Show this help");
    println!("  -v, --version          Show the CLI version");
    println!();
    println!("build options:");
    println!("  --target <name>        Select a target from the manifest");
    println!("  --emit <kind>          Artifact kind: executable, staticlib, or cdylib");
    println!("  --compiler <path>      Use a specific astra-compiler binary");
    println!("  --json                 Print a machine-readable build report");
    println!("  --no-download          Do not fetch a missing pinned C toolchain");
    println!();
    println!("doctor options:");
    println!("  --json                 Print a machine-readable diagnosis");
    println!("  --no-download          Do not fetch a missing pinned C toolchain");
    println!();
    println!("fmt options:");
    println!("  --check                Report formatting differences without writing");
    println!("  --json                 Print a machine-readable report");
    println!("  --compiler <path>      Use a specific astra-compiler binary");
    println!();
    println!("ast/explain options:");
    println!("  --json                 Print the full compiler response");
    println!("  --compiler <path>      Use a specific astra-compiler binary");
    println!("  --mode <mode>          core, app, web, or system (default: inferred)");
    println!("  --target <name>        Compiler target hint (default: js)");
    println!();
    println!("Exit codes:");
    println!("  0  success");
    println!("  1  invalid input, project error, or build/codegen failure");
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
    Ok(value.to_owned())
}

fn fail_unknown(flag: &str) -> ExitCode {
    eprintln!("astra-native: unknown flag '{flag}'");
    ExitCode::FAILURE
}

fn manifest_path(dir: &str, name: &str) -> String {
    std::path::Path::new(dir)
        .join(name)
        .to_string_lossy()
        .to_string()
}

fn manifest_code(diagnostics: &[Diagnostic]) -> &'static str {
    match diagnostics
        .first()
        .map(|diagnostic| diagnostic.code.as_str())
    {
        Some("NS2008") => "NS2008",
        Some("NS9001") => "NS9001",
        Some("NS9002") => "NS9002",
        Some("NS9003") => "NS9003",
        _ => "NS9002",
    }
}

fn load_project(dir: &str) -> Result<ProjectManifests, ManifestError> {
    let core_names: Vec<&str> = ["core.json", "core.toml"]
        .into_iter()
        .filter(|name| std::path::Path::new(&manifest_path(dir, name)).exists())
        .collect();
    if core_names.is_empty() {
        return Err(ManifestError {
            code: "NS9001",
            message: "No manifest found. Expected exactly one of core.json or core.toml."
                .to_owned(),
            path: dir.to_owned(),
        });
    }
    if core_names.len() > 1 {
        return Err(ManifestError {
            code: "NS9001",
            message: "Both core.json and core.toml exist. Exactly one manifest is allowed."
                .to_owned(),
            path: dir.to_owned(),
        });
    }
    let core_name = core_names[0].to_owned();
    let core_path = manifest_path(dir, &core_name);
    let core_text = fs::read_to_string(&core_path).map_err(|error| ManifestError {
        code: "NS9002",
        message: format!("Unable to read manifest: {error}"),
        path: core_path.clone(),
    })?;
    let core = if core_name.ends_with(".toml") {
        parse_core_toml(&core_text).map_err(|diagnostics| ManifestError {
            code: manifest_code(&diagnostics),
            message: diagnostics
                .first()
                .map(|diagnostic| diagnostic.message.clone())
                .unwrap_or_else(|| "Invalid core manifest.".to_owned()),
            path: core_path.clone(),
        })?
    } else {
        parse_core_json(&core_text).map_err(|diagnostics| ManifestError {
            code: manifest_code(&diagnostics),
            message: diagnostics
                .first()
                .map(|diagnostic| diagnostic.message.clone())
                .unwrap_or_else(|| "Invalid core manifest.".to_owned()),
            path: core_path.clone(),
        })?
    };

    let (build, build_present) = load_build_manifest(dir)?;

    Ok(ProjectManifests {
        root: std::path::PathBuf::from(dir),
        core_path,
        core_name: core_name.clone(),
        core_format: if core_name.ends_with(".toml") {
            "toml".to_owned()
        } else {
            "json".to_owned()
        },
        core,
        build,
        build_present,
    })
}

fn edition_for_path(path: &str) -> Result<Edition, ManifestError> {
    let mut dir = std::path::Path::new(path)
        .parent()
        .map(std::path::Path::to_path_buf)
        .unwrap_or_default();
    if dir.as_os_str().is_empty() {
        dir = std::path::PathBuf::from(".");
    }
    loop {
        if dir.join("core.json").exists() || dir.join("core.toml").exists() {
            return load_project(&dir.to_string_lossy()).map(|project| project.edition());
        }
        if !dir.pop() {
            break;
        }
    }
    Ok(Edition::V0_1)
}

fn load_build_manifest(dir: &str) -> Result<(BuildManifest, bool), ManifestError> {
    let build_names: Vec<&str> = ["build.json", "build.toml"]
        .into_iter()
        .filter(|name| std::path::Path::new(&manifest_path(dir, name)).exists())
        .collect();
    if build_names.len() > 1 {
        return Err(ManifestError {
            code: "NS9001",
            message: "Both build.json and build.toml exist. At most one is allowed.".to_owned(),
            path: dir.to_owned(),
        });
    }
    let Some(build_name) = build_names.first() else {
        return Ok((BuildManifest::default(), false));
    };
    let build_path = manifest_path(dir, build_name);
    let build_text = fs::read_to_string(&build_path).map_err(|error| ManifestError {
        code: "NS9002",
        message: format!("Unable to read manifest: {error}"),
        path: build_path.clone(),
    })?;
    let build = if build_name.ends_with(".toml") {
        parse_build_toml(&build_text).map_err(|diagnostics| ManifestError {
            code: manifest_code(&diagnostics),
            message: diagnostics
                .first()
                .map(|diagnostic| diagnostic.message.clone())
                .unwrap_or_else(|| "Invalid build manifest.".to_owned()),
            path: build_path.clone(),
        })?
    } else {
        parse_build_json(&build_text).map_err(|diagnostics| ManifestError {
            code: manifest_code(&diagnostics),
            message: diagnostics
                .first()
                .map(|diagnostic| diagnostic.message.clone())
                .unwrap_or_else(|| "Invalid build manifest.".to_owned()),
            path: build_path.clone(),
        })?
    };
    Ok((build, true))
}

pub(crate) fn manifest_reported(error: &ManifestError) -> ReportedDiagnostic {
    reported_json(
        error.code,
        "error",
        &error.message,
        None,
        Some(&error.path),
        None,
        None,
    )
}

fn emit_manifest_error(error: &ManifestError, json: bool) -> ExitCode {
    if json {
        let diagnostic = ManifestDiagnostic {
            code: error.code,
            severity: "error",
            message: error.message.clone(),
            path: error.path.clone(),
        };
        eprintln!(
            "{}",
            serde_json::to_string_pretty(&diagnostic).unwrap_or_default()
        );
    } else {
        print_diagnostics(&[manifest_reported(error)]);
    }
    ExitCode::FAILURE
}

pub(crate) fn reported_json(
    code: &str,
    severity: &str,
    message: &str,
    hint: Option<&str>,
    path: Option<&str>,
    line: Option<usize>,
    column: Option<usize>,
) -> ReportedDiagnostic {
    ReportedDiagnostic {
        code: code.to_owned(),
        severity: severity.to_owned(),
        message: message.to_owned(),
        hint: hint.map(str::to_owned),
        path: path.map(str::to_owned),
        line,
        column,
        related: Vec::new(),
        span: None,
        snippet: None,
    }
}

pub(crate) fn core_diagnostic(
    diagnostic: &Diagnostic,
    path: &str,
    text: &str,
) -> ReportedDiagnostic {
    let span = diagnostic.span.map(|span| (span.start, span.end));
    let (line, column) = match span {
        Some((start, _)) if start <= text.len() => {
            let (line, column) = byte_offset_to_line_col(text, start);
            (Some(line), Some(column))
        }
        _ => (None, None),
    };
    let related = diagnostic
        .related
        .iter()
        .map(|entry| {
            let same_file = entry.file.is_empty() || entry.file == path;
            let (line, column) = if same_file && entry.span.start <= text.len() {
                let (line, column) = byte_offset_to_line_col(text, entry.span.start);
                (Some(line), Some(column))
            } else {
                (None, None)
            };
            ReportedRelated {
                file: if entry.file.is_empty() {
                    path.to_owned()
                } else {
                    entry.file.clone()
                },
                start: entry.span.start,
                end: entry.span.end,
                message: entry.message.clone(),
                line,
                column,
            }
        })
        .collect();
    reported_json(
        &diagnostic.code,
        severity_name(diagnostic.severity),
        &diagnostic.message,
        diagnostic.hint.as_deref(),
        Some(path),
        line,
        column,
    )
    .with_span(span)
    .with_related(related)
}

pub(crate) fn print_core_diagnostics(diagnostics: &[Diagnostic], path: &str, text: &str) {
    let rendered: Vec<ReportedDiagnostic> = diagnostics
        .iter()
        .map(|diagnostic| core_diagnostic(diagnostic, path, text).with_snippet(text))
        .collect();
    print_diagnostics(&rendered);
}

fn print_reported(diagnostic: &ReportedDiagnostic) {
    eprintln!("{}", render::render(diagnostic));
}

pub(crate) fn print_diagnostics(diagnostics: &[ReportedDiagnostic]) {
    for diagnostic in diagnostics {
        print_reported(diagnostic);
    }
    if let Some(line) = render::summary(diagnostics) {
        eprintln!();
        eprintln!("{line}");
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum FfiDeclarationKind {
    Foreign,
    Script,
}

struct FfiDeclaration {
    kind: FfiDeclarationKind,
    name: String,
    span: Option<(usize, usize)>,
}

fn collect_ffi_declarations(items: &[Item], out: &mut Vec<FfiDeclaration>) {
    for item in items {
        match &item.kind {
            ItemKind::ForeignModule(foreign) => out.push(FfiDeclaration {
                kind: FfiDeclarationKind::Foreign,
                name: foreign.name.clone(),
                span: Some((foreign.span.start, foreign.span.end)),
            }),
            ItemKind::ScriptBlock(script) => out.push(FfiDeclaration {
                kind: FfiDeclarationKind::Script,
                name: script.name.clone(),
                span: Some((script.span.start, script.span.end)),
            }),
            ItemKind::ScopedItems(scoped) => collect_ffi_declarations(&scoped.items, out),
            _ => {}
        }
    }
}

fn ffi_mapping_problems(
    name: &str,
    mapping: Option<&serde_json::Value>,
    required: bool,
    active_target: &str,
) -> Vec<(String, &'static str)> {
    let Some(mapping) = mapping else {
        return if required {
            vec![(
                format!("Foreign module '{name}' has no manifest ffi mapping."),
                "Add an ffi entry for the module and its active target.",
            )]
        } else {
            Vec::new()
        };
    };
    let Some(mapping) = mapping.as_object() else {
        return vec![(
            format!("The manifest ffi mapping for '{name}' must be an object."),
            "Map the module to one or more target configuration objects.",
        )];
    };
    if mapping.is_empty() {
        return vec![(
            format!("The manifest ffi mapping for '{name}' is empty."),
            "Add at least one target mapping.",
        )];
    }
    if required && !mapping.contains_key(active_target) {
        return vec![(
            format!(
                "Foreign module '{name}' has no manifest ffi mapping for active target '{active_target}'."
            ),
            "Add an ffi entry for the active target or select a target with a mapping.",
        )];
    }

    let mut problems = Vec::new();
    for (target, target_mapping) in mapping {
        let Some(target_mapping) = target_mapping.as_object() else {
            problems.push((
                format!(
                    "The manifest ffi mapping for '{name}' target '{target}' must be an object."
                ),
                "Provide source and type fields for the target.",
            ));
            continue;
        };
        let source = target_mapping
            .get("source")
            .and_then(|value| value.as_str());
        let kind = target_mapping.get("type").and_then(|value| value.as_str());
        match source {
            Some(source) if !source.is_empty() => {}
            _ => problems.push((
                format!(
                    "The manifest ffi mapping for '{name}' target '{target}' needs a non-empty source."
                ),
                "Set source to a path, package specifier, or 'inline'.",
            )),
        }
        match kind {
            Some(kind) if !kind.is_empty() => {
                if kind.starts_with("inline-") && source != Some("inline") {
                    problems.push((
                        format!(
                            "The manifest ffi mapping for '{name}' target '{target}' uses '{kind}' but its source is not 'inline'."
                        ),
                        "Inline mappings use source: \"inline\".",
                    ));
                }
            }
            _ => problems.push((
                format!(
                    "The manifest ffi mapping for '{name}' target '{target}' needs a non-empty type."
                ),
                "Set type to a supported FFI mapping type.",
            )),
        }
        for field in ["ownership", "threading", "errors"] {
            let value = target_mapping.get(field).and_then(|value| value.as_str());
            if value.is_none_or(str::is_empty) {
                problems.push((
                    format!(
                        "The manifest ffi mapping for '{name}' target '{target}' needs a non-empty {field}."
                    ),
                    "State ownership, threading, and error behavior for every FFI mapping.",
                ));
            }
        }
    }
    problems
}

fn cmd_check_dir(dir: &str, json: bool, mode: Option<Mode>, _target: &str) -> ExitCode {
    match load_project(dir) {
        Ok(project) => project_check(&project, json, mode),
        Err(error) => emit_manifest_error(&error, json),
    }
}

fn resolve_project_entry(dir: &str) -> Result<String, ManifestError> {
    let project = load_project(dir)?;
    let entry = project.entry_value();
    let path = project.root.join(&entry);
    let display = display_path(&path.to_string_lossy());
    if !path.exists() {
        return Err(ManifestError {
            code: "NS9008",
            message: "Unable to read the file: the entry does not exist.".to_owned(),
            path: display,
        });
    }
    if !path.to_string_lossy().to_lowercase().ends_with(".ns") {
        return Err(ManifestError {
            code: "NS9008",
            message: "The project entry must be a .ns file.".to_owned(),
            path: display,
        });
    }
    Ok(path.to_string_lossy().to_string())
}

fn byte_offset_to_line_col(source: &str, offset: usize) -> (usize, usize) {
    let mut line = 1usize;
    let mut col = 1usize;
    let mut i = 0usize;
    for ch in source.chars() {
        if i >= offset {
            break;
        }
        if ch == '\n' {
            line += 1;
            col = 1;
        } else {
            col += 1;
        }
        i += ch.len_utf8();
    }
    (line, col)
}

fn discover_project_modules(
    root: &std::path::Path,
    entry_key: &str,
    entry_path: &std::path::Path,
) -> Vec<(String, String, String)> {
    fn visit(
        key: &str,
        abs_path: &std::path::Path,
        root: &std::path::Path,
        ordered: &mut Vec<(String, String, String)>,
        visited: &mut std::collections::HashSet<String>,
    ) {
        if !visited.insert(key.to_owned()) {
            return;
        }
        let text = match fs::read_to_string(abs_path) {
            Ok(text) => text,
            Err(_) => return,
        };
        for specifier in scan_import_specifiers(&text) {
            let Some(target_key) = resolve_project_specifier(key, &specifier, root) else {
                continue;
            };
            if visited.contains(&target_key) {
                continue;
            }
            let target_path = root.join(&target_key);
            if target_path.exists() {
                visit(&target_key, &target_path, root, ordered, visited);
            }
        }
        ordered.push((key.to_owned(), abs_path.to_string_lossy().to_string(), text));
    }

    let mut ordered = Vec::new();
    let mut visited = std::collections::HashSet::new();
    visit(entry_key, entry_path, root, &mut ordered, &mut visited);
    ordered
}

fn severity_name(severity: astra_core::diagnostic::Severity) -> &'static str {
    match severity {
        astra_core::diagnostic::Severity::Error => "error",
        astra_core::diagnostic::Severity::Warning => "warning",
        astra_core::diagnostic::Severity::Note => "note",
        astra_core::diagnostic::Severity::Help => "help",
    }
}

fn project_check(project: &ProjectManifests, json: bool, mode: Option<Mode>) -> ExitCode {
    let entry_value = project.entry_value();
    let entry_path = project.root.join(&entry_value);
    let mut ordered = Vec::new();
    if entry_path.exists()
        && entry_path.to_string_lossy().to_lowercase().ends_with(".ns")
        && let Ok(relative) = entry_path.strip_prefix(&project.root)
    {
        let entry_key = relative.to_string_lossy().replace('\\', "/");
        ordered = discover_project_modules(&project.root, &entry_key, &entry_path);
    }

    let target = project.source_check_target();
    let selected_mode = mode.or_else(|| project.default_mode());
    let sources: Vec<ProjectSourceUnit> = ordered
        .iter()
        .map(|(key, _path, text)| ProjectSourceUnit {
            key: key.clone(),
            path: key.clone(),
            text: text.clone(),
        })
        .collect();

    let response = if sources.is_empty() {
        None
    } else {
        let request = ProjectCheckRequest {
            api_version: API_VERSION.to_owned(),
            sources,
            target,
            edition: project.edition(),
            mode: selected_mode,
            integer_policy: project.integer_policy(),
        };
        Some(astra_compiler_api::check_project(&request))
    };

    let by_key: std::collections::BTreeMap<String, (String, String)> = ordered
        .iter()
        .map(|(key, path, text)| (key.clone(), (path.clone(), text.clone())))
        .collect();
    let mut flat = Vec::new();
    if let Some(response) = &response {
        for module in &response.modules {
            let (absolute_path, text) = by_key
                .get(&module.key)
                .cloned()
                .unwrap_or_else(|| (module.path.clone(), String::new()));
            for diagnostic in &module.diagnostics {
                let path = display_path(&absolute_path);
                flat.push(core_diagnostic(diagnostic, &path, &text).with_snippet(&text));
            }
        }
    }

    let ffi = project.core.ffi.as_ref();
    let invalid_ffi = ffi.is_some_and(|value| !value.is_object());
    if invalid_ffi {
        let path = display_path(&project.core_path);
        flat.push(reported_json(
            "NS7007",
            "error",
            "The manifest 'ffi' field must be an object.",
            Some("Map each foreign module name to its target configuration."),
            Some(&path),
            None,
            None,
        ));
    }
    if !invalid_ffi {
        let active_target = project.active_target();
        for (_key, absolute_path, text) in &ordered {
            let Ok(module) = parse_with_options(
                text,
                ParseOptions {
                    edition: project.edition(),
                },
            ) else {
                continue;
            };
            let mut declarations = Vec::new();
            collect_ffi_declarations(&module.items, &mut declarations);
            for declaration in declarations {
                let mapping = ffi.and_then(|value| value.get(&declaration.name));
                let problems = ffi_mapping_problems(
                    &declaration.name,
                    mapping,
                    declaration.kind == FfiDeclarationKind::Foreign,
                    &active_target,
                );
                for (message, hint) in problems {
                    let path = display_path(absolute_path);
                    flat.push(
                        reported_json(
                            "NS7007",
                            "error",
                            &message,
                            Some(hint),
                            Some(&path),
                            None,
                            None,
                        )
                        .with_span(declaration.span)
                        .with_snippet(text),
                    );
                }
            }
        }
    }

    let has_error = flat.iter().any(|diagnostic| diagnostic.severity == "error");
    let package = project.package_name();
    if json {
        let report = CheckReport {
            manifest: ManifestInfo {
                path: project.core_path.clone(),
                name: project.core_name.clone(),
                format: project.core_format.clone(),
            },
            package,
            ok: !has_error,
            diagnostics: flat,
        };
        println!(
            "{}",
            serde_json::to_string_pretty(&report).unwrap_or_default()
        );
    } else {
        if !has_error {
            println!("Manifest: {}", project.core_path);
            if let Some(package) = &package {
                println!("Package: {package}");
            }
        }
        print_diagnostics(&flat);
    }

    if has_error {
        ExitCode::FAILURE
    } else {
        ExitCode::SUCCESS
    }
}
fn cmd_doctor(args: &[String]) -> ExitCode {
    let mut dir: Option<String> = None;
    let mut json = false;
    let mut no_download = false;
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--json" => {
                json = true;
                i += 1;
            }
            "--no-download" => {
                no_download = true;
                i += 1;
            }
            "--compiler" => {
                if i + 1 >= args.len() {
                    eprintln!("astra-native: --compiler requires a value");
                    return ExitCode::FAILURE;
                }
                i += 2;
            }
            other if !other.starts_with('-') && dir.is_none() => {
                dir = Some(other.to_owned());
                i += 1;
            }
            other => return fail_unknown(other),
        }
    }
    let directory = std::path::absolute(dir.unwrap_or_else(|| ".".to_owned()))
        .map(|path| path.to_string_lossy().into_owned())
        .unwrap_or_else(|_| ".".to_owned());
    let (build, build_present) = match load_build_manifest(&directory) {
        Ok(value) => value,
        Err(error) => {
            if json {
                #[derive(serde::Serialize)]
                struct DoctorFailure {
                    ok: bool,
                    error: ReportedDiagnostic,
                }
                let failure = DoctorFailure {
                    ok: false,
                    error: reported_json(
                        error.code,
                        "error",
                        &error.message,
                        None,
                        Some(&error.path),
                        None,
                        None,
                    ),
                };
                println!(
                    "{}",
                    serde_json::to_string_pretty(&failure).unwrap_or_default()
                );
            } else {
                print_diagnostics(&[manifest_reported(&error)]);
            }
            return ExitCode::FAILURE;
        }
    };
    let build_data = build_present.then(|| serde_json::to_value(&build).unwrap_or_default());
    let policy = match parse_toolchain_policy(build_data.as_ref()) {
        Ok(policy) => policy,
        Err((code, message)) => {
            if json {
                #[derive(serde::Serialize)]
                struct DoctorFailure {
                    ok: bool,
                    error: ReportedDiagnostic,
                }
                let failure = DoctorFailure {
                    ok: false,
                    error: reported_json(&code, "error", &message, None, None, None, None),
                };
                println!(
                    "{}",
                    serde_json::to_string_pretty(&failure).unwrap_or_default()
                );
            } else {
                print_diagnostics(&[reported_json(
                    &code, "error", &message, None, None, None, None,
                )]);
            }
            return ExitCode::FAILURE;
        }
    };
    let astra_cc = std::env::var("ASTRA_CC").ok();
    let result = run_doctor(
        std::path::Path::new(&directory),
        &policy,
        astra_cc.as_deref(),
        &astra_toolchain::doctor::SystemProbe,
        !no_download,
    );
    let ok = result.winner.is_some();
    if json {
        #[derive(serde::Serialize)]
        struct DoctorReport {
            ok: bool,
            doctor: astra_toolchain::doctor::DoctorResult,
        }
        let out = DoctorReport { ok, doctor: result };
        println!("{}", serde_json::to_string_pretty(&out).unwrap_or_default());
    } else {
        println!("policy: {}", policy.request);
        for a in &result.attempts {
            println!("  {}: {}", a.rung, a.detail);
        }
        if let Some(w) = &result.winner {
            println!("resolved: {} ({}) at {}", w.id, w.family, w.path);
        } else if let Some(e) = &result.error {
            eprintln!("unresolved: {}", e);
        }
    }
    if ok {
        ExitCode::SUCCESS
    } else {
        ExitCode::FAILURE
    }
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
            "--compiler" => {
                if i + 1 >= args.len() {
                    eprintln!("astra-native: --compiler requires a value");
                    return ExitCode::FAILURE;
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
    let file = file.unwrap_or_else(|| ".".to_owned());
    let path = std::path::Path::new(&file);
    if path.is_dir() {
        return cmd_check_dir(&file, json, mode, &target);
    }
    let text = match read_file(&file) {
        Ok(t) => t,
        Err(code) => return code,
    };

    let edition = match edition_for_path(&file) {
        Ok(edition) => edition,
        Err(error) => return emit_manifest_error(&error, json),
    };
    let request = CompileRequest {
        api_version: API_VERSION.to_owned(),
        source: SourceUnit {
            path: file.clone(),
            text: text.clone(),
        },
        target,
        edition,
        mode,
        integer_policy: Default::default(),
    };
    let compiler = FrontendCompiler;
    let mut response = compiler.check(&request);

    if json {
        for diagnostic in &mut response.diagnostics {
            for related in &mut diagnostic.related {
                if related.file.is_empty() {
                    related.file = display_path(&file);
                }
            }
        }
        println!(
            "{}",
            serde_json::to_string_pretty(&response).unwrap_or_default()
        );
    } else {
        print_core_diagnostics(&response.diagnostics, &display_path(&file), &text);
    }

    if response.program.is_some() && response.diagnostics.is_empty() {
        ExitCode::SUCCESS
    } else {
        ExitCode::FAILURE
    }
}

fn display_path(path: &str) -> String {
    let cwd = std::env::current_dir().unwrap_or_default();
    if let Ok(rel) = std::path::Path::new(path).strip_prefix(&cwd) {
        rel.to_string_lossy().replace('\\', "/")
    } else {
        path.replace('\\', "/")
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
            "--compiler" => {
                if i + 1 >= args.len() {
                    eprintln!("astra-native: --compiler requires a value");
                    return ExitCode::FAILURE;
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
    let path = file.unwrap_or_else(|| ".".to_owned());

    let mut files: Vec<String> = Vec::new();
    let p = std::path::Path::new(&path);
    if p.is_file() {
        files.push(path.clone());
    } else if p.is_dir() {
        files.extend(discover_ns(&path));
        for name in ["core.json", "core.toml"] {
            let fp = std::path::Path::new(&path)
                .join(name)
                .to_string_lossy()
                .to_string();
            if std::path::Path::new(&fp).exists() {
                files.push(fp);
            }
        }
        files.sort();
    } else {
        files.push(path.clone());
    }

    let mut has_error = false;
    let mut has_diff = false;
    let mut file_reports: Vec<FmtFileReport> = Vec::new();
    let mut human_lines: Vec<String> = Vec::new();
    let mut rendered: Vec<ReportedDiagnostic> = Vec::new();

    for f in &files {
        let text = match read_file(f) {
            Ok(t) => t,
            Err(code) => {
                return code;
            }
        };

        let edition = match edition_for_path(f) {
            Ok(edition) => edition,
            Err(error) => return emit_manifest_error(&error, json),
        };

        let (formatted_opt, diagnostics) = format_one(f, &text, edition);

        if !diagnostics.is_empty() {
            has_error = true;
            if json {
                file_reports.push(FmtFileReport {
                    path: display_path(f),
                    status: "error".to_owned(),
                    diagnostics: diagnostics
                        .iter()
                        .map(|diagnostic| serde_json::json!(diagnostic.to_string()))
                        .collect(),
                });
            } else {
                rendered.extend(
                    diagnostics
                        .iter()
                        .map(|diagnostic| {
                            core_diagnostic(diagnostic, &display_path(f), &text).with_snippet(&text)
                        })
                        .collect::<Vec<ReportedDiagnostic>>(),
                );
            }
            continue;
        }

        let formatted = formatted_opt.unwrap();
        let changed = formatted != text;
        let status = if changed {
            if check { "would-reformat" } else { "formatted" }
        } else {
            "unchanged"
        };
        if changed {
            has_diff = true;
            if !check {
                let _ = fs::write(f, &formatted);
            }
        }

        if json {
            file_reports.push(FmtFileReport {
                path: display_path(f),
                status: status.to_owned(),
                diagnostics: Vec::new(),
            });
        } else {
            match status {
                "would-reformat" => human_lines.push(format!("would reformat {}", display_path(f))),
                "formatted" => human_lines.push(format!("formatted {}", display_path(f))),
                _ => {}
            }
        }
    }

    if json {
        let ok = !has_error && !(check && has_diff);
        let out = FmtReport {
            check,
            ok,
            files: file_reports,
            diagnostics: Vec::new(),
        };
        println!("{}", serde_json::to_string_pretty(&out).unwrap_or_default());
    } else {
        print_diagnostics(&rendered);
        if has_error {
        } else if files.is_empty() {
            println!("No formattable files found.");
        } else if !has_diff {
            println!("All {} file(s) are formatted.", files.len());
        } else if check {
            for l in &human_lines {
                println!("{}", l);
            }
            println!("{} file(s) need formatting.", human_lines.len());
        } else {
            for l in &human_lines {
                println!("{}", l);
            }
            println!("{} file(s) formatted.", human_lines.len());
        }
    }

    if has_error || (check && has_diff) {
        ExitCode::FAILURE
    } else {
        ExitCode::SUCCESS
    }
}

fn format_one(path: &str, text: &str, edition: Edition) -> (Option<String>, Vec<Diagnostic>) {
    let lower = path.to_lowercase();
    if lower.ends_with(".ns") {
        match format_source_with_options(text, FormatOptions { edition }) {
            Ok(f) => (Some(f), Vec::new()),
            Err(diagnostics) => (None, diagnostics),
        }
    } else if lower.ends_with("core.json") {
        match parse_core_json(text) {
            Ok(m) => (Some(format!("{}\n", format_core_json(&m))), Vec::new()),
            Err(diagnostics) => (None, diagnostics),
        }
    } else if lower.ends_with("build.json") {
        match parse_build_json(text) {
            Ok(m) => (Some(format!("{}\n", format_build_json(&m))), Vec::new()),
            Err(diagnostics) => (None, diagnostics),
        }
    } else if lower.ends_with("core.toml") {
        match format_core_toml(text) {
            Ok(formatted) => (Some(formatted), Vec::new()),
            Err(diagnostics) => (None, diagnostics),
        }
    } else if lower.ends_with("build.toml") {
        match format_build_toml(text) {
            Ok(formatted) => (Some(formatted), Vec::new()),
            Err(diagnostics) => (None, diagnostics),
        }
    } else {
        (Some(text.to_owned()), Vec::new())
    }
}

fn cmd_ast_dir(dir: &str, json: bool) -> ExitCode {
    let entry = match resolve_project_entry(dir) {
        Ok(entry) => entry,
        Err(error) => return emit_manifest_error(&error, json),
    };
    let text = match read_file(&entry) {
        Ok(text) => text,
        Err(code) => return code,
    };
    let edition = match edition_for_path(&entry) {
        Ok(edition) => edition,
        Err(error) => return emit_manifest_error(&error, json),
    };
    match parse_with_options(&text, ParseOptions { edition }) {
        Ok(module) => {
            if json {
                let module_value = serde_json::to_value(&module).unwrap_or_default();
                println!(
                    "{}",
                    serde_json::to_string_pretty(&NativeAstResponse {
                        api_version: API_VERSION.to_owned(),
                        ok: true,
                        ast: Some(module_value),
                        diagnostics: Vec::new(),
                    })
                    .unwrap_or_default()
                );
            } else {
                println!(
                    "{}",
                    serde_json::to_string_pretty(&serde_json::to_value(&module).unwrap())
                        .unwrap_or_default()
                );
            }
            ExitCode::SUCCESS
        }
        Err(diagnostics) => {
            print_core_diagnostics(&diagnostics, &display_path(&entry), &text);
            ExitCode::FAILURE
        }
    }
}

fn cmd_ast(args: &[String]) -> ExitCode {
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
            "--compiler" => {
                if i + 1 >= args.len() {
                    eprintln!("astra-native: --compiler requires a value");
                    return ExitCode::FAILURE;
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
    let _ = (&mode, &target);
    let file = file.unwrap_or_else(|| ".".to_owned());
    let path = std::path::Path::new(&file);
    if path.is_dir() {
        return cmd_ast_dir(&file, json);
    }
    let text = match read_file(&file) {
        Ok(t) => t,
        Err(code) => return code,
    };
    let edition = match edition_for_path(&file) {
        Ok(edition) => edition,
        Err(error) => return emit_manifest_error(&error, json),
    };

    match parse_with_options(&text, ParseOptions { edition }) {
        Ok(module) => {
            let module_value = serde_json::to_value(&module).unwrap_or_default();
            if json {
                println!(
                    "{}",
                    serde_json::to_string_pretty(&NativeAstResponse {
                        api_version: API_VERSION.to_owned(),
                        ok: true,
                        ast: Some(module_value.clone()),
                        diagnostics: Vec::new(),
                    })
                    .unwrap_or_default()
                );
            } else {
                println!(
                    "{}",
                    serde_json::to_string_pretty(&module_value).unwrap_or_default()
                );
            }
            ExitCode::SUCCESS
        }
        Err(diagnostics) => {
            print_core_diagnostics(&diagnostics, &display_path(&file), &text);
            if json {
                println!(
                    "{}",
                    serde_json::to_string_pretty(&NativeAstResponse {
                        api_version: API_VERSION.to_owned(),
                        ok: false,
                        ast: None,
                        diagnostics: diagnostics
                            .iter()
                            .map(|d| serde_json::json!(d.to_string()))
                            .collect(),
                    })
                    .unwrap_or_default()
                );
            }
            ExitCode::FAILURE
        }
    }
}

fn cmd_explain_dir(dir: &str, json: bool, _mode: Option<Mode>, _target: &str) -> ExitCode {
    let entry = match resolve_project_entry(dir) {
        Ok(entry) => entry,
        Err(error) => return emit_manifest_error(&error, json),
    };
    let text = match read_file(&entry) {
        Ok(text) => text,
        Err(code) => return code,
    };
    let edition = match edition_for_path(&entry) {
        Ok(edition) => edition,
        Err(error) => return emit_manifest_error(&error, json),
    };
    match parse_with_options(&text, ParseOptions { edition }) {
        Ok(module) => {
            let inference = infer_module_mode_named(&module, display_path(&entry));
            if json {
                let inference_value = serde_json::to_value(&inference).unwrap_or_default();
                println!(
                    "{}",
                    serde_json::to_string_pretty(&NativeExplainResponse {
                        api_version: API_VERSION.to_owned(),
                        ok: true,
                        inference: Some(inference_value),
                        explanation: Some(inference.explain()),
                        diagnostics: Vec::new(),
                    })
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
            print_core_diagnostics(&diagnostics, &display_path(&entry), &text);
            if json {
                println!(
                    "{}",
                    serde_json::to_string_pretty(&NativeExplainResponse {
                        api_version: API_VERSION.to_owned(),
                        ok: false,
                        inference: None,
                        explanation: None,
                        diagnostics: diagnostics
                            .iter()
                            .map(|diagnostic| serde_json::json!(diagnostic.to_string()))
                            .collect(),
                    })
                    .unwrap_or_default()
                );
            }
            ExitCode::FAILURE
        }
    }
}
fn is_error_code(value: &str) -> bool {
    let bytes = value.as_bytes();
    if bytes.len() != 6 || !bytes[2..].iter().all(u8::is_ascii_digit) {
        return false;
    }
    matches!(
        (bytes[0], bytes[1]),
        (b'N', b'S' | b'P' | b'R' | b'V') | (b'P', b'G')
    )
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
            "--compiler" => {
                if i + 1 >= args.len() {
                    eprintln!("astra-native: --compiler requires a value");
                    return ExitCode::FAILURE;
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
    let _ = (&mode, &target);
    let file = file.unwrap_or_else(|| ".".to_owned());
    if is_error_code(&file) {
        eprintln!(
            "error: explanations are not bundled in this build; see spec-v0.1.md §14 or spec-v0.2.md §6"
        );
        return ExitCode::FAILURE;
    }
    let path = std::path::Path::new(&file);
    if path.is_dir() {
        return cmd_explain_dir(&file, json, mode, &target);
    }
    let text = match read_file(&file) {
        Ok(t) => t,
        Err(code) => return code,
    };
    let edition = match edition_for_path(&file) {
        Ok(edition) => edition,
        Err(error) => return emit_manifest_error(&error, json),
    };

    match parse_with_options(&text, ParseOptions { edition }) {
        Ok(module) => {
            let inference = infer_module_mode_named(&module, display_path(&file));
            let inference_value = serde_json::to_value(&inference).unwrap_or_default();
            if json {
                println!(
                    "{}",
                    serde_json::to_string_pretty(&NativeExplainResponse {
                        api_version: API_VERSION.to_owned(),
                        ok: true,
                        inference: Some(inference_value),
                        explanation: Some(inference.explain()),
                        diagnostics: Vec::new(),
                    })
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
            print_core_diagnostics(&diagnostics, &display_path(&file), &text);
            if json {
                println!(
                    "{}",
                    serde_json::to_string_pretty(&NativeExplainResponse {
                        api_version: API_VERSION.to_owned(),
                        ok: false,
                        inference: None,
                        explanation: None,
                        diagnostics: diagnostics
                            .iter()
                            .map(|d| serde_json::json!(d.to_string()))
                            .collect(),
                    })
                    .unwrap_or_default()
                );
            }
            ExitCode::FAILURE
        }
    }
}
