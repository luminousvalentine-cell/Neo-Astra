use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode};

use astra_compiler_api::{
    API_VERSION, CompileRequest, FrontendCompiler, ProjectCompileRequest, ProjectSourceUnit,
    SourceUnit,
};
use astra_core::mode::Mode;
use astra_manifest::TargetSection;

use crate::{ProjectManifests, ReportedDiagnostic, display_path, load_project};

#[derive(serde::Serialize)]
struct BuildReport {
    target: String,
    backend: String,
    entry: String,
    output: Option<String>,
    diagnostics: Vec<ReportedDiagnostic>,
}

#[derive(Default)]
struct BuildOptions {
    project: Option<String>,
    target: Option<String>,
    emit: Option<String>,
    json: bool,
    no_download: bool,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum GeneratedKind {
    Source,
    Header,
    Host,
    Script,
}

struct GeneratedFile {
    filename: String,
    content: String,
    kind: GeneratedKind,
    is_entry: bool,
}

#[derive(Clone)]
struct HostCompiler {
    bin: String,
    args: Vec<String>,
    family: String,
}

pub fn cmd_build(args: &[String]) -> ExitCode {
    let options = match parse_args(args) {
        Ok(options) => options,
        Err(diagnostic) => return emit_failure("(default)", "js", "", *diagnostic, false),
    };

    match run_build(&options) {
        Ok(report) => {
            if options.json {
                print_report(&report);
            } else {
                print_diagnostics(&report.diagnostics);
                if let Some(output) = &report.output {
                    println!("built {} -> {}", report.target, output);
                }
            }
            if report
                .diagnostics
                .iter()
                .any(|diagnostic| diagnostic.severity == "error")
            {
                ExitCode::FAILURE
            } else {
                ExitCode::SUCCESS
            }
        }
        Err(report) => {
            if options.json {
                print_report(&report);
            } else {
                print_diagnostics(&report.diagnostics);
            }
            ExitCode::FAILURE
        }
    }
}

fn parse_args(args: &[String]) -> Result<BuildOptions, Box<ReportedDiagnostic>> {
    let mut options = BuildOptions::default();
    let mut positional = Vec::new();
    let mut index = 0;
    while index < args.len() {
        let arg = &args[index];
        match arg.as_str() {
            "--" => {
                positional.extend(args[index + 1..].iter().cloned());
                break;
            }
            "--json" => {
                options.json = true;
                index += 1;
            }
            "--no-download" => {
                options.no_download = true;
                index += 1;
            }
            "--target" | "--emit" | "--compiler" => {
                let Some(value) = args.get(index + 1) else {
                    return Err(option_error(arg));
                };
                if value.starts_with('-') {
                    return Err(option_error(arg));
                }
                match arg.as_str() {
                    "--target" => options.target = Some(value.clone()),
                    "--emit" => options.emit = Some(value.clone()),
                    "--compiler" => {}
                    _ => unreachable!(),
                }
                index += 2;
            }
            other if other.starts_with('-') => return Err(option_error(other)),
            other => {
                positional.push(other.to_owned());
                index += 1;
            }
        }
    }
    if positional.len() > 1 {
        return Err(Box::new(diag(
            "NS9000",
            "Expected at most one project directory.",
            None,
        )));
    }
    options.project = positional.into_iter().next();
    Ok(options)
}

fn option_error(option: &str) -> Box<ReportedDiagnostic> {
    Box::new(diag(
        "NS9000",
        &format!("The {option} option requires a value."),
        None,
    ))
}

fn run_build(options: &BuildOptions) -> Result<BuildReport, BuildReport> {
    let fatal = |diagnostic| fatal_report(options, diagnostic);
    let project_dir = options.project.clone().unwrap_or_else(|| ".".to_owned());
    let project =
        load_project(&project_dir).map_err(|error| fatal(crate::manifest_reported(&error)))?;
    let target_name = options
        .target
        .clone()
        .or_else(|| env::var("ASTRA_TARGET").ok())
        .or_else(|| match project.build.build.as_ref() {
            Some(build) => build.default_target.clone(),
            None => Some("native".to_owned()),
        })
        .filter(|target| !target.is_empty())
        .ok_or_else(|| {
            fatal(diag(
                "NS9011",
                "No build target was selected. Set --target, ASTRA_TARGET, or build.defaultTarget.",
                Some(&display_path(&project.core_path)),
            ))
        })?;

    if !project.build.targets.contains_key(&target_name)
        && !matches!(target_name.as_str(), "c" | "native" | "ts")
    {
        return Err(fatal(diag(
            "NS9011",
            &format!("Target '{target_name}' is not declared in the manifest."),
            Some(&display_path(&project.core_path)),
        )));
    }

    let target_config = project
        .build
        .targets
        .get(&target_name)
        .cloned()
        .unwrap_or_default();
    let backend = target_config
        .backend
        .clone()
        .unwrap_or_else(|| match target_name.as_str() {
            "c" | "native" | "ts" => target_name.clone(),
            _ => "js".to_owned(),
        });

    if backend != "c" && backend != "native" {
        if backend == "js" || backend == "ts" {
            return run_js_ts_bridge(&project, &target_name, &backend, options);
        }
        return Err(fatal(diag(
            "NS8001",
            &format!(
                "Target '{target_name}' uses backend '{backend}'; native build currently supports 'c', 'native', 'js', and 'ts' backends."
            ),
            Some(&display_path(&project.core_path)),
        )));
    }

    let emit_value = options
        .emit
        .clone()
        .or_else(|| target_extra_string(&target_config, "emit"))
        .unwrap_or_else(|| "executable".to_owned());
    let emit_kind = match emit_value.as_str() {
        "executable" => "executable",
        "library" | "lib" | "staticlib" => "staticlib",
        "cdylib" => "cdylib",
        other => {
            return Err(fatal(diag(
                "NS9012",
                &format!(
                    "Unknown emit kind '{other}'. Use executable, staticlib, cdylib, or library."
                ),
                Some(&display_path(&project.core_path)),
            )));
        }
    };
    let library_mode = emit_kind != "executable";

    let entry_value = build_entry(&project).map_err(|diagnostic| fatal(*diagnostic))?;
    let entry_path = project.root.join(&entry_value);
    let entry_key = entry_path
        .strip_prefix(&project.root)
        .map(|path| path.to_string_lossy().replace('\\', "/"))
        .unwrap_or_default();
    if entry_key.is_empty() || entry_key == ".." || entry_key.starts_with("../") {
        return Err(fatal(diag(
            "NS9013",
            "The configured entry point must be inside the project.",
            Some(&display_path(&entry_path.to_string_lossy())),
        )));
    }
    if !entry_path.to_string_lossy().to_lowercase().ends_with(".ns") {
        return Err(fatal(diag(
            "NS9013",
            "The configured entry point must be a .ns file.",
            Some(&display_path(&entry_path.to_string_lossy())),
        )));
    }
    if !entry_path.exists() {
        let message = match fs::read(&entry_path) {
            Ok(_) => "Unable to read the project entry.".to_owned(),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                format!(
                    "Unable to read the file: ENOENT: no such file or directory, open '{}'",
                    entry_path.to_string_lossy()
                )
            }
            Err(error) => format!("Unable to read the file: {error}"),
        };
        return Err(fatal(diag(
            "NS9008",
            &message,
            Some(&display_path(&entry_path.to_string_lossy())),
        )));
    }

    let ordered = crate::discover_project_modules(&project.root, &entry_key, &entry_path);
    let entry_text = ordered
        .iter()
        .find(|(key, _, _)| key == &entry_key)
        .map(|(_, _, text)| text.clone())
        .unwrap_or_default();

    let mode = target_config
        .mode
        .as_deref()
        .and_then(Mode::parse)
        .or_else(|| project.default_mode());
    if target_config.mode.is_some() && mode.is_none() {
        return Err(report_error(
            &target_name,
            &backend,
            &entry_key,
            diag(
                "NS9012",
                &format!(
                    "Unknown mode '{}' for target '{target_name}'.",
                    target_config.mode.as_deref().unwrap_or_default()
                ),
                Some(&display_path(&project.core_path)),
            ),
        ));
    }
    let integer_policy = project.integer_policy();

    let mut files = Vec::new();
    let mut diagnostics = Vec::new();
    if ordered.len() <= 1 && !library_mode {
        let request = CompileRequest {
            api_version: API_VERSION.to_owned(),
            source: SourceUnit {
                path: entry_key.clone(),
                text: entry_text.clone(),
            },
            target: "c".to_owned(),
            edition: project.edition(),
            mode,
            integer_policy,
        };
        let response = FrontendCompiler.compile(&request);
        diagnostics.extend(
            response
                .diagnostics
                .iter()
                .map(|diagnostic| compiler_diagnostic(diagnostic, &entry_path, &entry_text)),
        );
        for artifact in &response.artifacts {
            let Some(filename) = artifact
                .get("name")
                .or_else(|| artifact.get("filename"))
                .and_then(|value| value.as_str())
            else {
                continue;
            };
            let Some(content) = artifact.get("content").and_then(|value| value.as_str()) else {
                continue;
            };
            let kind = match artifact.get("type").and_then(|value| value.as_str()) {
                Some("c-source") => GeneratedKind::Source,
                Some("c-host-shim") => GeneratedKind::Host,
                Some("c-script") => GeneratedKind::Script,
                _ => continue,
            };
            files.push(GeneratedFile {
                filename: filename.to_owned(),
                content: content.to_owned(),
                kind,
                is_entry: kind == GeneratedKind::Source,
            });
        }
    } else {
        let request = ProjectCompileRequest {
            api_version: API_VERSION.to_owned(),
            sources: ordered
                .iter()
                .map(|(key, _, text)| ProjectSourceUnit {
                    key: key.clone(),
                    path: key.clone(),
                    text: text.clone(),
                })
                .collect(),
            target: "c".to_owned(),
            edition: project.edition(),
            mode,
            integer_policy,
            library: library_mode,
            import_map: Default::default(),
        };
        let response = astra_compiler_api::compile_project(&request);
        diagnostics.extend(
            response
                .diagnostics
                .iter()
                .map(|diagnostic| compiler_diagnostic_unattributed(diagnostic, &entry_path)),
        );
        for module in &response.modules {
            files.push(GeneratedFile {
                filename: module.c_filename.clone(),
                content: module.c_source.clone(),
                kind: GeneratedKind::Source,
                is_entry: module.key == entry_key,
            });
            if !module.h_source.is_empty() {
                files.push(GeneratedFile {
                    filename: module.h_filename.clone(),
                    content: module.h_source.clone(),
                    kind: GeneratedKind::Header,
                    is_entry: false,
                });
            }
            for (filename, content) in &module.script_sources {
                files.push(GeneratedFile {
                    filename: filename.clone(),
                    content: content.clone(),
                    kind: GeneratedKind::Script,
                    is_entry: false,
                });
            }
        }
        if !response.host_shim.is_empty() {
            files.push(GeneratedFile {
                filename: "astra_host.c".to_owned(),
                content: response.host_shim,
                kind: GeneratedKind::Host,
                is_entry: false,
            });
        }
    }

    if diagnostics
        .iter()
        .any(|diagnostic| diagnostic.severity == "error")
    {
        return Err(BuildReport {
            target: target_name,
            backend,
            entry: entry_key,
            output: None,
            diagnostics,
        });
    }

    let out_dir = output_dir(&project, &target_config, &target_name);
    if let Err(error) = fs::create_dir_all(&out_dir) {
        return Err(fatal(diag(
            "NS9014",
            &format!("Unable to create the output directory: {error}"),
            Some(&display_path(&out_dir.to_string_lossy())),
        )));
    }

    let mut c_sources = Vec::new();
    let mut scripts = Vec::new();
    let mut header_paths = Vec::new();
    let mut main_c_path: Option<PathBuf> = None;
    let mut host_c_path: Option<PathBuf> = None;
    for file in &files {
        let path = out_dir.join(&file.filename);
        if let Some(parent) = path.parent()
            && let Err(error) = fs::create_dir_all(parent)
        {
            return Err(fatal(diag(
                "NS9014",
                &format!("Unable to create generated output directories: {error}"),
                Some(&display_path(&out_dir.to_string_lossy())),
            )));
        }
        if let Err(error) = fs::write(&path, &file.content) {
            return Err(fatal(diag(
                "NS9014",
                &format!("Unable to write generated C artifacts: {error}"),
                Some(&display_path(&path.to_string_lossy())),
            )));
        }
        match file.kind {
            GeneratedKind::Source => {
                c_sources.push(path.clone());
                if file.is_entry {
                    main_c_path = Some(path);
                }
            }
            GeneratedKind::Host => host_c_path = Some(path),
            GeneratedKind::Script => scripts.push(path),
            GeneratedKind::Header => header_paths.push(path),
        }
    }

    let output_path = if library_mode {
        build_library(
            &project,
            &target_config,
            &target_name,
            emit_kind,
            &out_dir,
            &c_sources,
            &scripts,
            &header_paths,
            host_c_path.as_deref(),
            !options.no_download,
        )
        .map_err(fatal)?
    } else if backend == "native" {
        let Some(main_c) = main_c_path.as_ref() else {
            return Err(fatal(diag(
                "NS8002",
                "Compiler did not produce the expected C artifacts for native compilation.",
                Some(&display_path(&out_dir.to_string_lossy())),
            )));
        };
        let Some(host_c) = host_c_path.as_ref() else {
            return Err(fatal(diag(
                "NS8002",
                "Compiler did not produce the expected host shim for native compilation.",
                Some(&display_path(&out_dir.to_string_lossy())),
            )));
        };
        let _ = main_c;
        let exe_name = if cfg!(windows) { "main.exe" } else { "main" };
        let exe_path = out_dir.join(exe_name);
        let build_data = serde_json::to_value(&project.build).unwrap_or_default();
        let toolchain_policy = astra_toolchain::policy::parse_toolchain_policy(Some(&build_data))
            .map_err(|(code, message)| {
            fatal(diag(
                &code,
                &message,
                Some(&display_path(&project.core_path)),
            ))
        })?;
        let astra_cc = env::var("ASTRA_CC").ok();
        let toolchain_root =
            std::path::absolute(&project.root).unwrap_or_else(|_| project.root.clone());
        let compiler = resolve_host_compiler(
            &toolchain_root,
            &toolchain_policy,
            astra_cc.as_deref(),
            !options.no_download,
        )
        .map_err(|error| fatal(diag(&error.code, &error.message, None)))?;
        let mut args = vec![
            "-std=c17".to_owned(),
            "-Wall".to_owned(),
            "-Wno-unused-variable".to_owned(),
        ];
        args.extend(cflags(&target_config));
        args.push("-o".to_owned());
        args.push(exe_path.to_string_lossy().to_string());
        for source in &c_sources {
            args.push(source.to_string_lossy().to_string());
        }
        args.push(host_c.to_string_lossy().to_string());
        for script in &scripts {
            args.push(script.to_string_lossy().to_string());
        }
        if scripts
            .iter()
            .any(|path| path.to_string_lossy().to_lowercase().ends_with(".cpp"))
        {
            args.push("-lstdc++".to_owned());
        }
        run_compiler(&compiler, &args, &project.root).map_err(|error| {
            fatal(diag(
                "NS9015",
                &format!("Native compilation failed with {}:\n{error}", compiler.bin),
                Some(&display_path(&out_dir.to_string_lossy())),
            ))
        })?;
        exe_path
    } else {
        main_c_path.unwrap_or_else(|| out_dir.clone())
    };

    Ok(BuildReport {
        target: target_name,
        backend,
        entry: entry_key,
        output: Some(display_path(&output_path.to_string_lossy())),
        diagnostics,
    })
}

#[allow(clippy::too_many_arguments)]
#[allow(clippy::result_large_err)]
fn build_library(
    project: &ProjectManifests,
    target_config: &TargetSection,
    target_name: &str,
    emit_kind: &str,
    out_dir: &Path,
    c_sources: &[PathBuf],
    scripts: &[PathBuf],
    header_paths: &[PathBuf],
    host_c_path: Option<&Path>,
    allow_download: bool,
) -> Result<PathBuf, ReportedDiagnostic> {
    let out_display = display_path(&out_dir.to_string_lossy());
    let Some(host_c) = host_c_path else {
        return Err(diag(
            "NS8002",
            "Compiler did not produce the expected host shim for library emission.",
            Some(&out_display),
        ));
    };
    if c_sources.is_empty() {
        return Err(diag(
            "NS8002",
            "Compiler did not produce the expected C artifacts for library emission.",
            Some(&out_display),
        ));
    }

    let build_data = serde_json::to_value(&project.build).unwrap_or_default();
    let policy = astra_toolchain::policy::parse_toolchain_policy(Some(&build_data)).map_err(
        |(code, message)| diag(&code, &message, Some(&display_path(&project.core_path))),
    )?;
    let astra_cc = env::var("ASTRA_CC").ok();
    let toolchain_root =
        std::path::absolute(&project.root).unwrap_or_else(|_| project.root.clone());
    let compiler = resolve_host_compiler(
        &toolchain_root,
        &policy,
        astra_cc.as_deref(),
        allow_download,
    )
    .map_err(|error| diag(&error.code, &error.message, None))?;

    let cflags = cflags(target_config);
    let obj_dir = out_dir.join("obj");
    fs::create_dir_all(&obj_dir).map_err(|error| {
        diag(
            "NS9014",
            &format!("Unable to create the object directory: {error}"),
            Some(&display_path(&obj_dir.to_string_lossy())),
        )
    })?;

    let is_cdylib = emit_kind == "cdylib";
    let pic = is_cdylib && !cfg!(windows) && compiler_flavor(&compiler, false) != "msvc";
    let mut objects = Vec::new();
    for source in c_sources.iter().chain(scripts.iter()) {
        let stem = source
            .file_stem()
            .map(|stem| stem.to_string_lossy().into_owned())
            .unwrap_or_else(|| "source".to_owned());
        let object = obj_dir.join(format!("{stem}.o"));
        compile_library_object(
            &compiler,
            source,
            &object,
            &cflags,
            pic,
            &project.root,
            out_dir,
        )?;
        objects.push(object);
    }

    let package_name = project
        .package_name()
        .unwrap_or_else(|| target_name.to_owned());
    let sanitized: String = package_name
        .chars()
        .map(|character| {
            if character.is_ascii_alphanumeric() || matches!(character, '_' | '.' | '-') {
                character
            } else {
                '_'
            }
        })
        .collect();
    let lib_stem = format!("lib{sanitized}");
    let has_cpp = scripts.iter().any(|path| is_cpp(path));

    if emit_kind == "staticlib" {
        let archive = out_dir.join(format!("{lib_stem}.a"));
        let mut failures = Vec::new();
        let mut built = false;
        for candidate in archiver_candidates(&compiler) {
            let _ = fs::remove_file(&archive);
            let mut args = candidate.prefix.clone();
            args.push("rcs".to_owned());
            args.push(archive.to_string_lossy().into_owned());
            args.extend(
                objects
                    .iter()
                    .map(|object| object.to_string_lossy().into_owned()),
            );
            match run_tool(&candidate.bin, &args, &project.root) {
                Ok(()) => {
                    built = true;
                    break;
                }
                Err(error) => failures.push(format!("{}: {error}", candidate.bin)),
            }
        }
        if !built {
            let _ = fs::remove_file(&archive);
            return Err(diag(
                "NS9102",
                &format!(
                    "No archiver (zig ar, ar, llvm-ar, or gcc-ar) could build a static library:\n{}",
                    failures.join("\n")
                ),
                Some(&out_display),
            ));
        }
        return Ok(archive);
    }

    let host_object = obj_dir.join("astra_host.o");
    compile_library_object(
        &compiler,
        host_c,
        &host_object,
        &cflags,
        pic,
        &project.root,
        out_dir,
    )?;
    let shared_ext = if cfg!(windows) { ".dll" } else { ".so" };
    let shared = out_dir.join(format!("{lib_stem}{shared_ext}"));
    let mut link_args = vec![
        "-shared".to_owned(),
        "-o".to_owned(),
        shared.to_string_lossy().into_owned(),
    ];
    link_args.extend(
        objects
            .iter()
            .map(|object| object.to_string_lossy().into_owned()),
    );
    link_args.push(host_object.to_string_lossy().into_owned());

    let flavor = compiler_flavor(&compiler, has_cpp);
    if flavor == "msvc" {
        let mut exports = Vec::new();
        for header in header_paths {
            for name in exported_symbols_from_header(header) {
                if !exports.contains(&name) {
                    exports.push(name);
                }
            }
        }
        if !exports.is_empty() {
            let def_path = out_dir.join(format!("{lib_stem}.def"));
            let mut def = String::from("EXPORTS\n");
            for name in exports {
                def.push_str(&name);
                def.push('\n');
            }
            if let Err(error) = fs::write(&def_path, def) {
                return Err(diag(
                    "NS9014",
                    &format!("Unable to write the module-definition file: {error}"),
                    Some(&out_display),
                ));
            }
            link_args.push(format!("-Wl,/DEF:{}", def_path.to_string_lossy()));
        }
        link_args.push(format!(
            "-Wl,/IMPLIB:{}",
            out_dir.join(format!("{lib_stem}.lib")).to_string_lossy()
        ));
    } else if cfg!(windows) {
        link_args.push("-Wl,--export-all-symbols".to_owned());
        link_args.push(format!(
            "-Wl,--out-implib,{}",
            out_dir.join(format!("{lib_stem}.dll.a")).to_string_lossy()
        ));
    }
    if has_cpp && flavor == "gnu" {
        link_args.push("-lstdc++".to_owned());
    }

    let mut args = invocation_args(&compiler, has_cpp);
    args.extend(link_args);
    run_tool(&compiler.bin, &args, &project.root).map_err(|error| {
        let label = compiler_label(&compiler, has_cpp);
        diag(
            "NS9015",
            &format!("Shared library link failed with {label}:\n{error}"),
            Some(&out_display),
        )
    })?;
    Ok(shared)
}

#[allow(clippy::result_large_err)]
fn compile_library_object(
    compiler: &HostCompiler,
    source: &Path,
    object: &Path,
    cflags: &[String],
    pic: bool,
    cwd: &Path,
    out_dir: &Path,
) -> Result<(), ReportedDiagnostic> {
    let cpp = is_cpp(source);
    let mut args = invocation_args(compiler, cpp);
    args.push(if cpp { "-std=c++17" } else { "-std=c17" }.to_owned());
    args.push("-Wall".to_owned());
    args.push("-Wno-unused-variable".to_owned());
    if pic {
        args.push("-fPIC".to_owned());
    }
    args.extend(cflags.iter().cloned());
    args.push("-c".to_owned());
    args.push("-o".to_owned());
    args.push(object.to_string_lossy().into_owned());
    args.push(source.to_string_lossy().into_owned());
    run_tool(&compiler.bin, &args, cwd).map_err(|error| {
        let name = source
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_default();
        diag(
            "NS9015",
            &format!(
                "Library object compilation failed with {} for {name}:\n{error}",
                compiler_label(compiler, cpp)
            ),
            Some(&display_path(&out_dir.to_string_lossy())),
        )
    })
}

fn is_cpp(path: &Path) -> bool {
    path.extension()
        .is_some_and(|extension| extension.eq_ignore_ascii_case("cpp"))
}

fn invocation_args(compiler: &HostCompiler, cpp: bool) -> Vec<String> {
    let mut args = compiler.args.clone();
    if compiler.family == "zig" && cpp {
        for arg in &mut args {
            if arg == "cc" {
                *arg = "c++".to_owned();
            }
        }
    }
    args
}

fn compiler_label(compiler: &HostCompiler, cpp: bool) -> String {
    if compiler.family == "zig" {
        format!("{} {}", compiler.bin, if cpp { "c++" } else { "cc" })
    } else {
        compiler.bin.clone()
    }
}

struct Archiver {
    bin: String,
    prefix: Vec<String>,
}

fn archiver_candidates(compiler: &HostCompiler) -> Vec<Archiver> {
    let mut candidates = Vec::new();
    if compiler.family == "zig" {
        candidates.push(Archiver {
            bin: compiler.bin.clone(),
            prefix: vec!["ar".to_owned()],
        });
    } else if let Ok(output) = Command::new(&compiler.bin)
        .args(&compiler.args)
        .arg("-print-prog-name=ar")
        .output()
        && output.status.success()
    {
        let value = String::from_utf8_lossy(&output.stdout).trim().to_owned();
        if !value.is_empty() {
            candidates.push(Archiver {
                bin: value,
                prefix: Vec::new(),
            });
        }
    }
    for name in ["llvm-ar", "gcc-ar", "ar"] {
        if !candidates.iter().any(|candidate| candidate.bin == name) {
            candidates.push(Archiver {
                bin: name.to_owned(),
                prefix: Vec::new(),
            });
        }
    }
    candidates
}

fn compiler_flavor(compiler: &HostCompiler, cpp: bool) -> &'static str {
    let mut command = Command::new(&compiler.bin);
    command
        .args(invocation_args(compiler, cpp))
        .arg("-dumpmachine");
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(0x0800_0000);
    }
    match command.output() {
        Ok(output)
            if output.status.success()
                && String::from_utf8_lossy(&output.stdout).contains("windows-msvc") =>
        {
            "msvc"
        }
        _ => "gnu",
    }
}

fn exported_symbols_from_header(header: &Path) -> Vec<String> {
    const MARKER: &str = "#define NS_ASTRA_EXPORTED_PROTOTYPES 1";
    let Ok(text) = fs::read_to_string(header) else {
        return Vec::new();
    };
    let Some(index) = text.find(MARKER) else {
        return Vec::new();
    };
    let mut names = Vec::new();
    for line in text[index + MARKER.len()..].lines() {
        let line = line.trim();
        if !line.ends_with(';') {
            continue;
        }
        let Some(paren) = line.find('(') else {
            continue;
        };
        let before = line[..paren].trim_end();
        let name = before
            .split(|character: char| !(character.is_ascii_alphanumeric() || character == '_'))
            .next_back()
            .unwrap_or("");
        if !name.is_empty() && !names.iter().any(|existing| existing == name) {
            names.push(name.to_owned());
        }
    }
    names
}

fn run_tool(bin: &str, args: &[String], cwd: &Path) -> Result<(), String> {
    let mut command = Command::new(bin);
    command.args(args).current_dir(cwd);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(0x0800_0000);
    }
    let output = command.output().map_err(|error| error.to_string())?;
    if output.status.success() {
        return Ok(());
    }
    let stderr = String::from_utf8_lossy(&output.stderr).to_string();
    Err(if stderr.trim().is_empty() {
        format!("exit {}", output.status)
    } else {
        stderr
    })
}

fn build_entry(project: &ProjectManifests) -> Result<String, Box<ReportedDiagnostic>> {
    let Some(nova) = project.build.nova.as_ref() else {
        return Ok("src/main.ns".to_owned());
    };
    nova.entry
        .clone()
        .filter(|entry| !entry.is_empty())
        .ok_or_else(|| {
            Box::new(diag(
                "NV0002",
                "The manifest does not declare nova.entry.",
                Some(&display_path(&project.core_path)),
            ))
        })
}

fn output_dir(project: &ProjectManifests, target: &TargetSection, target_name: &str) -> PathBuf {
    target
        .out_dir
        .as_ref()
        .or_else(|| {
            project
                .build
                .build
                .as_ref()
                .and_then(|build| build.out_dir.as_ref())
        })
        .map(|path| project.root.join(path))
        .unwrap_or_else(|| project.root.join("dist").join(target_name))
}

fn target_extra_string(target: &TargetSection, key: &str) -> Option<String> {
    target
        .extra
        .get(key)
        .and_then(|value| value.as_str())
        .map(str::to_owned)
}

fn cflags(target: &TargetSection) -> Vec<String> {
    let mut flags = Vec::new();
    if let Some(values) = target
        .extra
        .get("cflags")
        .and_then(|value| value.as_array())
    {
        for value in values {
            if let Some(flag) = value.as_str() {
                flags.push(flag.to_owned());
            }
        }
    }
    if let Some(value) = target
        .extra
        .get("optLevel")
        .or_else(|| target.extra.get("optimization"))
    {
        let value = value
            .as_str()
            .map(str::to_owned)
            .or_else(|| value.as_i64().map(|number| number.to_string()));
        if let Some(value) = value {
            let level = value.strip_prefix('-').unwrap_or(&value);
            let level = level.strip_prefix('O').unwrap_or(level);
            flags.push(format!("-O{level}"));
        }
    }
    if let Some(triple) = target
        .triple
        .clone()
        .or_else(|| target_extra_string(target, "targetTriple"))
    {
        flags.push(format!("--target={triple}"));
    }
    flags
}

fn compiler_diagnostic(
    diagnostic: &astra_core::diagnostic::Diagnostic,
    path: &Path,
    text: &str,
) -> ReportedDiagnostic {
    crate::core_diagnostic(diagnostic, &display_path(&path.to_string_lossy()), text)
        .with_snippet(text)
}

fn compiler_diagnostic_unattributed(
    diagnostic: &astra_core::diagnostic::Diagnostic,
    path: &Path,
) -> ReportedDiagnostic {
    crate::core_diagnostic(diagnostic, &display_path(&path.to_string_lossy()), "")
}

fn diag(code: &str, message: &str, path: Option<&str>) -> ReportedDiagnostic {
    crate::reported_json(code, "error", message, None, path, None, None)
}

fn fatal_report(options: &BuildOptions, diagnostic: ReportedDiagnostic) -> BuildReport {
    BuildReport {
        target: options
            .target
            .clone()
            .unwrap_or_else(|| "(default)".to_owned()),
        backend: "js".to_owned(),
        entry: String::new(),
        output: None,
        diagnostics: vec![diagnostic],
    }
}

fn report_error(
    target: &str,
    backend: &str,
    entry: &str,
    diagnostic: ReportedDiagnostic,
) -> BuildReport {
    BuildReport {
        target: target.to_owned(),
        backend: backend.to_owned(),
        entry: entry.to_owned(),
        output: None,
        diagnostics: vec![diagnostic],
    }
}

fn emit_failure(
    target: &str,
    backend: &str,
    entry: &str,
    diagnostic: ReportedDiagnostic,
    json: bool,
) -> ExitCode {
    let report = report_error(target, backend, entry, diagnostic);
    if json {
        print_report(&report);
    } else {
        print_diagnostics(&report.diagnostics);
    }
    ExitCode::FAILURE
}

fn print_report(report: &BuildReport) {
    println!(
        "{}",
        serde_json::to_string_pretty(report).unwrap_or_default()
    );
}

fn print_diagnostics(diagnostics: &[ReportedDiagnostic]) {
    crate::print_diagnostics(diagnostics);
}

fn resolve_host_compiler(
    project_root: &Path,
    policy: &astra_toolchain::policy::ToolchainPolicy,
    env_cc: Option<&str>,
    allow_download: bool,
) -> Result<HostCompiler, astra_toolchain::doctor::ToolchainError> {
    let resolution = astra_toolchain::doctor::resolve_with_download(
        project_root,
        policy,
        env_cc,
        &astra_toolchain::doctor::SystemProbe,
        allow_download,
    )?;
    let (bin, args) = astra_toolchain::doctor::compiler_invocation(&resolution);
    Ok(HostCompiler {
        bin,
        args,
        family: resolution.family,
    })
}

fn run_compiler(compiler: &HostCompiler, args: &[String], cwd: &Path) -> Result<(), String> {
    let output = Command::new(&compiler.bin)
        .args(&compiler.args)
        .args(args)
        .current_dir(cwd)
        .output()
        .map_err(|error| error.to_string())?;
    if output.status.success() {
        return Ok(());
    }
    let stderr = String::from_utf8_lossy(&output.stderr).to_string();
    Err(if stderr.is_empty() {
        format!("exit {}", output.status)
    } else {
        stderr
    })
}

fn find_node_executable() -> Option<PathBuf> {
    if let Ok(path) = env::var("ASTRA_NODE")
        && !path.trim().is_empty()
    {
        let candidate = PathBuf::from(path.trim());
        if candidate.is_file() || candidate.exists() {
            return Some(candidate);
        }
    }

    if let Ok(path_var) = env::var("PATH") {
        for dir in env::split_paths(&path_var) {
            let candidate = dir.join(if cfg!(windows) { "node.exe" } else { "node" });
            if candidate.is_file() {
                return Some(candidate);
            }
        }
    }

    None
}

fn find_orchestrator_cli() -> Option<PathBuf> {
    if let Ok(exe) = env::current_exe()
        && let Some(dir) = exe.parent()
    {
        for rel in [
            "lib/cli/dist/index.js",
            "bridge/cli/dist/index.js",
            "cli/dist/index.js",
        ] {
            let candidate = dir.join("..").join(rel);
            if candidate.is_file() {
                return Some(candidate);
            }
            let candidate = dir.join(rel);
            if candidate.is_file() {
                return Some(candidate);
            }
        }
    }
    if let Ok(root) = env::var("ASTRA_REPO_ROOT") {
        let candidate = PathBuf::from(root)
            .join("orchestrator")
            .join("packages")
            .join("cli")
            .join("dist")
            .join("index.js");
        if candidate.is_file() {
            return Some(candidate);
        }
    }

    if let Ok(exe) = env::current_exe() {
        let mut cur = exe.parent();
        while let Some(dir) = cur {
            let candidate = dir
                .join("orchestrator")
                .join("packages")
                .join("cli")
                .join("dist")
                .join("index.js");
            if candidate.is_file() {
                return Some(candidate);
            }
            cur = dir.parent();
        }
    }

    if let Ok(cwd) = env::current_dir() {
        let mut cur = Some(cwd.as_path());
        while let Some(dir) = cur {
            let candidate = dir
                .join("orchestrator")
                .join("packages")
                .join("cli")
                .join("dist")
                .join("index.js");
            if candidate.is_file() {
                return Some(candidate);
            }
            cur = dir.parent();
        }
    }

    None
}

fn run_js_ts_bridge(
    project: &crate::ProjectManifests,
    target_name: &str,
    backend: &str,
    options: &BuildOptions,
) -> Result<BuildReport, BuildReport> {
    let fatal = |diagnostic| fatal_report(options, diagnostic);

    let node_bin = match find_node_executable() {
        Some(bin) => bin,
        None => {
            return Err(fatal(diag(
                "NS9015",
                "Node.js runtime not found on PATH or ASTRA_NODE. Building for JS/TS targets requires Node.js.",
                Some(&display_path(&project.core_path)),
            )));
        }
    };

    let cli_js = match find_orchestrator_cli() {
        Some(script) => script,
        None => {
            return Err(fatal(diag(
                "NS9015",
                "Orchestrator CLI script (orchestrator/packages/cli/dist/index.js) not found for JS/TS build delegation.",
                Some(&display_path(&project.core_path)),
            )));
        }
    };

    let mut cmd = Command::new(&node_bin);
    cmd.arg(&cli_js).arg("build");

    let proj_dir_str = options.project.as_deref().unwrap_or(".");
    cmd.arg(proj_dir_str);

    cmd.arg("--target").arg(target_name);

    if let Some(emit) = &options.emit {
        cmd.arg("--emit").arg(emit);
    }
    if options.no_download {
        cmd.arg("--no-download");
    }

    cmd.arg("--json");

    if let Ok(cwd) = env::current_dir() {
        cmd.current_dir(&cwd);
    }

    if env::var("ASTRA_COMPILER_BIN").is_err()
        && let Ok(exe) = env::current_exe()
        && let Some(dir) = exe.parent()
    {
        for rel in [
            "../bin/astra-compiler",
            "bin/astra-compiler",
            "astra-compiler",
        ] {
            let candidate = dir.join(rel);
            let candidate = if cfg!(windows) {
                candidate.with_extension("exe")
            } else {
                candidate
            };
            if candidate.is_file() {
                cmd.env("ASTRA_COMPILER_BIN", &candidate);
                break;
            }
        }
    }
    if env::var("ASTRA_TSC_BIN").is_err()
        && let Ok(exe) = env::current_exe()
        && let Some(dir) = exe.parent()
    {
        for rel in ["../typescript/tsc", "typescript/tsc", "tsc"] {
            let candidate = dir.join(rel);
            let candidate = if cfg!(windows) {
                candidate.with_extension("exe")
            } else {
                candidate
            };
            if candidate.is_file() {
                cmd.env("ASTRA_TSC_BIN", &candidate);
                break;
            }
        }
    }

    let output = cmd.output().map_err(|err| {
        fatal(diag(
            "NS9015",
            &format!("Failed to execute JS/TS build bridge: {err}"),
            Some(&display_path(&project.core_path)),
        ))
    })?;

    let stdout_str = String::from_utf8_lossy(&output.stdout);
    if let Ok(report) = serde_json::from_str::<serde_json::Value>(&stdout_str) {
        let target = report
            .get("target")
            .and_then(|v| v.as_str())
            .unwrap_or(target_name)
            .to_owned();
        let backend_val = report
            .get("backend")
            .and_then(|v| v.as_str())
            .unwrap_or(backend)
            .to_owned();
        let entry = report
            .get("entry")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_owned();
        let out = report
            .get("output")
            .and_then(|v| v.as_str())
            .map(str::to_owned);
        let mut diags = Vec::new();
        if let Some(diag_array) = report.get("diagnostics").and_then(|v| v.as_array()) {
            for d in diag_array {
                if let Ok(rd) = serde_json::from_value::<ReportedDiagnostic>(d.clone()) {
                    diags.push(rd);
                }
            }
        }
        let build_report = BuildReport {
            target,
            backend: backend_val,
            entry,
            output: out,
            diagnostics: diags,
        };
        if output.status.success() {
            Ok(build_report)
        } else {
            Err(build_report)
        }
    } else {
        let stderr_str = String::from_utf8_lossy(&output.stderr);
        Err(fatal(diag(
            "NS9015",
            format!("JS/TS build bridge failed: {}\n{}", stdout_str, stderr_str).trim(),
            Some(&display_path(&project.core_path)),
        )))
    }
}
