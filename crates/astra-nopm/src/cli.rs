use std::fs;
use std::path::PathBuf;

use serde::Serialize;

use crate::command::SystemRunner;
use crate::error::NopmError;
use crate::http::{HttpClient, UreqClient, encode_uri_component};
use crate::json_order::{OrderedJson, js_string};
use crate::lockfile::{
    LOCKFILE_NAME, Lockfile, ToolchainRecord, assert_lockfile_current, create_lockfile,
    read_lockfile_file, write_lockfile_file,
};
use crate::manifest::add_dependency;
use crate::manifest::find_core_manifest;
use crate::manifest::remove_dependency;
use crate::package_id::{PackageId, parse_package_id};
use crate::project::{ProjectManifest, absolute_path, read_project};
use crate::resolve::{collect_descriptors, resolve_for_targets};
use crate::store::{install_locked_packages, store_index_path};
use crate::sys::{self, SysContext};

const VERSION: &str = env!("CARGO_PKG_VERSION");

const HELP: &str = "NoPM universal package manager

Usage:
  nopm <command> [options]

Commands:
  add <id>[@version] [dir] [--json] [--dry-run]
                              Add a dependency; use sys:<tool> for git/cmake/clang
  remove <name> [dir] [--json] [--dry-run]
                              Remove a dependency and update the lockfile
  lock [dir] [--json]         Resolve dependencies and write core.lock.json
  tree [dir] [--json]        Print the locked dependency package list
  install [dir] [--locked|--frozen] [--json]
                             Verify and install locked packages
  search <query> [--registry <url>] [--json]
                             Search a registry gateway
  audit <id> [--registry <url>] [--json]
                             Read registry audit metadata for a package
  why <name>                 Show packages that depend on a package
  update [dir] [--json]      Re-resolve the lockfile
  version                    Print the NoPM version
  help                       Show this help

Current adapters:
  path, git, npm, cargo, pypi, nuget, oci

Staged adapters:
  none

Exit codes:
  0  success
  1  package, lockfile, or installation failure
  2  usage error
";

#[derive(Clone, Debug)]
struct Options {
    command: String,
    directory: PathBuf,
    target: Option<String>,
    registry: Option<String>,
    json: bool,
    locked: bool,
    frozen: bool,
    dry_run: bool,
}

pub fn run(args: &[String]) -> i32 {
    let json = args.iter().any(|argument| argument == "--json");
    match run_inner(args) {
        Ok(code) => code,
        Err(error) => emit_error(&error, json),
    }
}

fn run_inner(args: &[String]) -> Result<i32, NopmError> {
    let options = parse_args(args)?;
    match options.command.as_str() {
        "help" => {
            print!("{HELP}");
            Ok(0)
        }
        "version" => {
            println!("nopm {VERSION}");
            Ok(0)
        }
        "add" => run_add(&options),
        "remove" => run_remove(&options),
        "lock" => run_lock(&options),
        "update" => {
            let mut options = options.clone();
            options.command = "update".to_owned();
            run_lock(&options)
        }
        "tree" => run_tree(&options),
        "install" => run_install(&options),
        "why" => run_why(&options),
        "search" => run_search(&options),
        "audit" => run_audit(&options),
        other => Err(usage(format!("Unknown command '{other}'."))),
    }
}

fn usage(message: impl Into<String>) -> NopmError {
    NopmError::new("NP0012", message)
}

fn current_directory() -> PathBuf {
    std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."))
}

fn base_options(
    command: &str,
    json: bool,
    locked: bool,
    frozen: bool,
    dry_run: bool,
    registry: Option<String>,
) -> Options {
    Options {
        command: command.to_owned(),
        directory: current_directory(),
        target: None,
        registry,
        json,
        locked,
        frozen,
        dry_run,
    }
}

fn parse_args(args: &[String]) -> Result<Options, NopmError> {
    let json = args.iter().any(|argument| argument == "--json");
    let locked = args.iter().any(|argument| argument == "--locked");
    let frozen = args.iter().any(|argument| argument == "--frozen");
    let dry_run = args.iter().any(|argument| argument == "--dry-run");
    let mut positional: Vec<String> = Vec::new();
    let mut registry: Option<String> = None;

    let mut index = 0;
    while index < args.len() {
        let argument = &args[index];
        if argument == "--json"
            || argument == "--locked"
            || argument == "--frozen"
            || argument == "--dry-run"
        {
            index += 1;
            continue;
        }
        if argument == "--registry" {
            match args.get(index + 1) {
                Some(value) if !value.starts_with('-') => {
                    registry = Some(value.clone());
                    index += 2;
                    continue;
                }
                _ => return Err(usage("The --registry option requires a base URL.")),
            }
        }
        if argument == "-h" || argument == "--help" {
            return Ok(base_options(
                "help", json, locked, frozen, dry_run, registry,
            ));
        }
        if argument == "-v" || argument == "--version" {
            return Ok(base_options(
                "version", json, locked, frozen, dry_run, registry,
            ));
        }
        if argument.starts_with('-') {
            return Err(usage(format!("Unknown option '{argument}'.")));
        }
        positional.push(argument.clone());
        index += 1;
    }

    let Some(command) = positional.first().cloned() else {
        return Ok(base_options(
            "help", json, locked, frozen, dry_run, registry,
        ));
    };
    let rest = &positional[1..];
    let is_sys = rest
        .first()
        .map(|argument| argument.starts_with("sys:"))
        .unwrap_or(false);
    let max_positionals = if (command == "add" || command == "remove") && !is_sys {
        2
    } else {
        1
    };
    if rest.len() > max_positionals {
        return Err(usage(format!(
            "nopm {command} accepts at most {max_positionals} argument(s)."
        )));
    }
    if locked && frozen {
        return Err(usage("Use only one of --locked or --frozen."));
    }

    if command == "add" || command == "remove" {
        return Ok(Options {
            command,
            directory: absolute_path(&PathBuf::from(
                rest.get(1).map(String::as_str).unwrap_or("."),
            )),
            target: rest.first().cloned(),
            registry,
            json,
            locked,
            frozen,
            dry_run,
        });
    }

    if matches!(command.as_str(), "search" | "audit" | "why") {
        return Ok(Options {
            command,
            directory: current_directory(),
            target: rest.first().cloned(),
            registry,
            json,
            locked,
            frozen,
            dry_run,
        });
    }

    Ok(Options {
        command,
        directory: absolute_path(&PathBuf::from(
            rest.first().map(String::as_str).unwrap_or("."),
        )),
        target: None,
        registry,
        json,
        locked,
        frozen,
        dry_run,
    })
}

fn add_alias(name: &str) -> String {
    let base = name.split('@').next().unwrap_or(name);
    let mut alias: String = base
        .chars()
        .map(|character| {
            if character.is_ascii_alphanumeric() || character == '_' || character == '-' {
                character
            } else {
                '-'
            }
        })
        .collect();
    if alias.starts_with('-') {
        alias.replace_range(0..1, "_");
    }
    alias
}

fn read_optional_file(path: &std::path::Path) -> Option<String> {
    fs::read_to_string(path).ok()
}

fn requests_for(project: &ProjectManifest) -> Vec<crate::package_id::DependencyRequest> {
    project
        .dependencies
        .iter()
        .map(|dependency| dependency.request.clone())
        .collect()
}

fn build_and_write_lockfile(
    project: &ProjectManifest,
    lockfile_path: &std::path::Path,
) -> Result<Lockfile, NopmError> {
    let collected =
        collect_descriptors(project, requests_for(project), &UreqClient, &SystemRunner)?;
    let resolution = resolve_for_targets(
        &collected.requests,
        &collected.descriptors,
        &project.targets,
    )?;
    let toolchain = resolve_project_toolchain(project)?;
    let lockfile = create_lockfile(
        &resolution,
        &project.manifest_hash,
        project.ffi.clone(),
        toolchain,
    )?;
    write_lockfile_file(lockfile_path, &lockfile)?;
    Ok(lockfile)
}

fn resolve_project_toolchain(
    project: &ProjectManifest,
) -> Result<Option<ToolchainRecord>, NopmError> {
    use astra_toolchain::doctor::{SystemProbe, resolve_with_download};
    use astra_toolchain::policy::parse_toolchain_policy;

    let policy = parse_toolchain_policy(Some(&project.build_data))
        .map_err(|(code, message)| NopmError::new(code, message))?;
    let env_cc = std::env::var("ASTRA_CC").ok();
    match resolve_with_download(
        &project.directory,
        &policy,
        env_cc.as_deref(),
        &SystemProbe,
        true,
    ) {
        Ok(winner) => Ok(Some(ToolchainRecord {
            rung: winner.rung,
            id: winner.id,
            family: winner.family,
            path: winner.path,
            version: Some(winner.version),
        })),
        Err(error) => {
            if policy.request == "auto" && error.code == "NS9102" {
                Ok(None)
            } else {
                Err(NopmError::new(error.code, error.message))
            }
        }
    }
}

fn run_lock(options: &Options) -> Result<i32, NopmError> {
    let project = read_project(&options.directory, true)?;
    let lockfile_path = project.directory.join(LOCKFILE_NAME);
    let lockfile = build_and_write_lockfile(&project, &lockfile_path)?;

    if options.json {
        let response = LockResponse {
            ok: true,
            command: &options.command,
            lockfile: lockfile_path.to_string_lossy().into_owned(),
            packages: lockfile.packages.len(),
            targets: lockfile.targets.keys().cloned().collect(),
        };
        println!(
            "{}",
            serde_json::to_string_pretty(&response).unwrap_or_default()
        );
    } else {
        println!(
            "Locked {} package(s) for {} target(s).",
            lockfile.packages.len(),
            lockfile.targets.len()
        );
    }
    Ok(0)
}

fn run_add(options: &Options) -> Result<i32, NopmError> {
    let Some(id) = options.target.as_deref() else {
        return Err(usage("nopm add requires a package ID."));
    };
    if id.starts_with("sys:") {
        return run_add_sys(options, id);
    }

    let parsed = parse_package_id(id)?;
    let alias = add_alias(&parsed.name);
    let spec = id;
    let project = read_project(&options.directory, false)?;
    if project
        .dependencies
        .iter()
        .any(|dependency| dependency.alias == alias)
    {
        return Err(NopmError::with_path(
            "NP0013",
            format!("Dependency '{alias}' already exists."),
            alias,
        ));
    }

    if options.dry_run {
        if options.json {
            let response = AddDryRunResponse {
                ok: true,
                dry_run: true,
                action: "add",
                id: spec,
            };
            println!("{}", serde_json::to_string(&response).unwrap_or_default());
        } else {
            println!(
                "Would add {spec} to {}",
                project.directory.to_string_lossy()
            );
        }
        return Ok(0);
    }

    let core_file = find_core_manifest(&project.directory)?;
    let original_manifest = fs::read_to_string(&core_file.path).map_err(|error| {
        NopmError::with_path(
            "NS9002",
            format!("Unable to read manifest: {error}"),
            core_file.display_path(),
        )
    })?;
    let lockfile_path = project.directory.join(LOCKFILE_NAME);
    let original_lockfile = read_optional_file(&lockfile_path);

    let mut manifest_written = false;
    let mut lockfile_written = false;
    let mut written_packages = 0usize;
    let outcome = (|| -> Result<(), NopmError> {
        add_dependency(&core_file, &alias, spec)?;
        manifest_written = true;

        let new_project = read_project(&options.directory, false)?;
        let lockfile = build_and_write_lockfile(&new_project, &lockfile_path)?;
        lockfile_written = true;
        written_packages = lockfile.packages.len();
        install_locked_packages(&new_project.directory, &lockfile, &UreqClient)?;
        Ok(())
    })();

    if let Err(error) = outcome {
        if manifest_written {
            let _ = fs::write(&core_file.path, &original_manifest);
        }
        if lockfile_written {
            match &original_lockfile {
                Some(source) => {
                    let _ = fs::write(&lockfile_path, source);
                }
                None => {
                    let _ = fs::remove_file(&lockfile_path);
                }
            }
        }
        return Err(error);
    }

    if options.json {
        let response = AddResponse {
            ok: true,
            action: "add",
            id: spec,
            packages: written_packages,
        };
        println!("{}", serde_json::to_string(&response).unwrap_or_default());
    } else {
        println!("Added {spec} (alias: {alias}).");
    }
    Ok(0)
}

fn run_add_sys(options: &Options, id: &str) -> Result<i32, NopmError> {
    let tool = id.strip_prefix("sys:").unwrap_or("");
    sys::assert_known_sys_tool(tool)?;
    let platform = sys::current_platform();
    let context = SysContext {
        platform: platform.clone(),
        uid: sys::current_uid(),
        managers: sys::detect_managers(&platform, &SystemRunner),
    };
    let runner = SystemRunner;

    if sys::detect_installed(tool, &runner) {
        emit_sys_install(options, tool, None, &[], true, options.dry_run)?;
        return Ok(0);
    }

    let plan = sys::plan_sys_install(tool, &context)?;
    if options.dry_run {
        emit_sys_install(
            options,
            tool,
            plan.manager.as_deref(),
            &plan.commands,
            false,
            true,
        )?;
        return Ok(0);
    }

    let result = sys::execute_sys_install(tool, &context, &runner)?;
    emit_sys_install(
        options,
        tool,
        result.manager.as_deref(),
        &result.commands,
        result.installed,
        false,
    )?;
    Ok(0)
}

fn emit_sys_install(
    options: &Options,
    tool: &str,
    manager: Option<&str>,
    commands: &[Vec<String>],
    installed: bool,
    dry_run: bool,
) -> Result<(), NopmError> {
    if options.json {
        let response = SysInstallResponse {
            ok: true,
            dry_run: dry_run.then_some(true),
            action: "sys-install",
            tool,
            manager,
            commands,
            installed,
        };
        println!("{}", serde_json::to_string(&response).unwrap_or_default());
        return Ok(());
    }
    if installed {
        println!("already installed");
        return Ok(());
    }
    if dry_run {
        for command in commands {
            println!("{}", command.join(" "));
        }
        return Ok(());
    }
    println!(
        "Installed {tool} via {}.",
        manager.unwrap_or("unknown manager")
    );
    Ok(())
}

fn run_remove(options: &Options) -> Result<i32, NopmError> {
    let Some(name) = options.target.as_deref() else {
        return Err(usage("nopm remove requires a package name."));
    };
    if name.starts_with("sys:") {
        return Err(NopmError::with_path(
            "NP0016",
            "NoPM does not uninstall system tools.",
            name,
        ));
    }

    let project = read_project(&options.directory, false)?;
    if !project
        .dependencies
        .iter()
        .any(|dependency| dependency.alias == name)
    {
        return Err(NopmError::with_path(
            "NP0014",
            format!("Dependency '{name}' is not declared."),
            name,
        ));
    }

    if options.dry_run {
        if options.json {
            let response = RemoveDryRunResponse {
                ok: true,
                dry_run: true,
                action: "remove",
                name,
            };
            println!("{}", serde_json::to_string(&response).unwrap_or_default());
        } else {
            println!(
                "Would remove {name} from {}",
                project.directory.to_string_lossy()
            );
        }
        return Ok(0);
    }

    let core_file = find_core_manifest(&project.directory)?;
    let original_manifest = fs::read_to_string(&core_file.path).map_err(|error| {
        NopmError::with_path(
            "NS9002",
            format!("Unable to read manifest: {error}"),
            core_file.display_path(),
        )
    })?;
    let lockfile_path = project.directory.join(LOCKFILE_NAME);
    let original_lockfile = read_optional_file(&lockfile_path);

    let mut manifest_written = false;
    let mut lockfile_written = false;
    let outcome = (|| -> Result<(), NopmError> {
        if !remove_dependency(&core_file, name)? {
            return Err(NopmError::with_path(
                "NP0014",
                format!("Dependency '{name}' is not declared."),
                name,
            ));
        }
        manifest_written = true;

        let new_project = read_project(&options.directory, false)?;
        let lockfile = build_and_write_lockfile(&new_project, &lockfile_path)?;
        lockfile_written = true;
        install_locked_packages(&new_project.directory, &lockfile, &UreqClient)?;
        Ok(())
    })();

    if let Err(error) = outcome {
        if manifest_written {
            let _ = fs::write(&core_file.path, &original_manifest);
        }
        if lockfile_written {
            match &original_lockfile {
                Some(source) => {
                    let _ = fs::write(&lockfile_path, source);
                }
                None => {
                    let _ = fs::remove_file(&lockfile_path);
                }
            }
        }
        return Err(error);
    }

    if options.json {
        let response = RemoveResponse {
            ok: true,
            action: "remove",
            name,
        };
        println!("{}", serde_json::to_string(&response).unwrap_or_default());
    } else {
        println!("Removed {name}.");
    }
    Ok(0)
}

fn current_lockfile(project: &ProjectManifest, options: &Options) -> Result<Lockfile, NopmError> {
    let path = project.directory.join(LOCKFILE_NAME);
    let lockfile = read_lockfile_file(&path)?;
    if options.locked || options.frozen {
        assert_lockfile_current(&lockfile, &project.manifest_hash)?;
    } else if lockfile.manifest_hash != project.manifest_hash {
        return Err(NopmError::with_path(
            "NP0009",
            "Lockfile is stale. Run `nopm lock` before installing.",
            LOCKFILE_NAME,
        ));
    }
    Ok(lockfile)
}

fn run_tree(options: &Options) -> Result<i32, NopmError> {
    let project = read_project(&options.directory, true)?;
    let lockfile = current_lockfile(&project, options)?;

    if options.json {
        let response = TreeResponse {
            ok: true,
            command: "tree",
            packages: lockfile
                .packages
                .iter()
                .map(TreePackage::from_package)
                .collect(),
        };
        println!(
            "{}",
            serde_json::to_string_pretty(&response).unwrap_or_default()
        );
    } else {
        for entry in &lockfile.packages {
            let targets = if entry.targets.is_empty() {
                "universal".to_owned()
            } else {
                entry.targets.join(", ")
            };
            println!("{} [{}]", entry.id, targets);
            for dependency in &entry.dependencies {
                println!("  -> {dependency}");
            }
        }
    }
    Ok(0)
}

fn run_install(options: &Options) -> Result<i32, NopmError> {
    let project = read_project(&options.directory, true)?;
    let lockfile = current_lockfile(&project, options)?;
    let store = install_locked_packages(&project.directory, &lockfile, &UreqClient)?;

    if options.json {
        let response = InstallResponse {
            ok: true,
            command: "install",
            packages: store.packages.len(),
            store: store_index_path(&project.directory)
                .to_string_lossy()
                .into_owned(),
        };
        println!(
            "{}",
            serde_json::to_string_pretty(&response).unwrap_or_default()
        );
    } else {
        println!("Installed {} package(s).", store.packages.len());
    }
    Ok(0)
}

fn run_why(options: &Options) -> Result<i32, NopmError> {
    let Some(target) = options.target.as_deref() else {
        return Err(usage("nopm why requires a package name."));
    };
    let project = read_project(&options.directory, true)?;
    let lockfile = current_lockfile(&project, options)?;
    let matches: Vec<String> = lockfile
        .packages
        .iter()
        .filter(|entry| {
            entry
                .dependencies
                .iter()
                .any(|dependency| dependency_matches(dependency, target))
        })
        .map(|entry| entry.id.clone())
        .collect();

    if options.json {
        let response = WhyResponse {
            ok: true,
            command: "why",
            target,
            packages: matches,
        };
        println!(
            "{}",
            serde_json::to_string_pretty(&response).unwrap_or_default()
        );
    } else if matches.is_empty() {
        println!("No locked package depends on {target}.");
    } else {
        for entry in matches {
            println!("{entry} depends on {target}");
        }
    }
    Ok(0)
}

fn run_search(options: &Options) -> Result<i32, NopmError> {
    let Some(query) = options.target.as_deref() else {
        return Err(usage("nopm search requires a search query."));
    };
    let registry = registry_for(options)?;
    let url = format!("{}/v1/search?q={}", registry, encode_uri_component(query));
    let client = UreqClient;
    let response = client
        .get(&url, &[("Accept", "application/json")])
        .map_err(|message| NopmError::new("NP0010", message))?;
    if !response.ok() {
        return Err(NopmError::with_path(
            "NP0011",
            format!("Registry search failed: HTTP {}.", response.status),
            registry,
        ));
    }
    let value =
        OrderedJson::parse(&response.body).map_err(|message| NopmError::new("NP0010", message))?;

    if options.json {
        let wrapper = OrderedJson::Object(vec![
            ("ok".to_owned(), OrderedJson::Bool(true)),
            (
                "command".to_owned(),
                OrderedJson::String("search".to_owned()),
            ),
            ("result".to_owned(), value),
        ]);
        println!("{}", wrapper.to_pretty());
        return Ok(0);
    }

    let Some(results) = value.get("results").and_then(OrderedJson::as_array) else {
        return Err(NopmError::with_path(
            "NP0011",
            "Registry returned an invalid search response.",
            registry,
        ));
    };
    for result in results {
        if result.is_object() {
            println!(
                "{}@{}",
                js_string(result.get("name")),
                js_string(result.get("version"))
            );
        }
    }
    Ok(0)
}

fn run_audit(options: &Options) -> Result<i32, NopmError> {
    let Some(target) = options.target.as_deref() else {
        return Err(usage("nopm audit requires a package ID."));
    };
    let id: PackageId = parse_package_id(target)?;
    let Some(version) = id.version.as_deref() else {
        return Err(usage("nopm audit requires an exact package version."));
    };
    let registry = registry_for(options)?;
    let url = format!(
        "{}/v1/audit/{}/{}",
        registry,
        encode_uri_component(&id.name),
        encode_uri_component(version)
    );
    let client = UreqClient;
    let response = client
        .get(&url, &[("Accept", "application/json")])
        .map_err(|message| NopmError::new("NP0010", message))?;
    if !response.ok() {
        return Err(NopmError::with_path(
            "NP0011",
            format!("Registry audit failed: HTTP {}.", response.status),
            registry,
        ));
    }
    let value =
        OrderedJson::parse(&response.body).map_err(|message| NopmError::new("NP0010", message))?;

    if options.json {
        let wrapper = OrderedJson::Object(vec![
            ("ok".to_owned(), OrderedJson::Bool(true)),
            (
                "command".to_owned(),
                OrderedJson::String("audit".to_owned()),
            ),
            ("result".to_owned(), value),
        ]);
        println!("{}", wrapper.to_pretty());
    } else if value.is_object() {
        let yanked = matches!(value.get("yanked"), Some(OrderedJson::Bool(true)));
        let advisories = value
            .get("advisories")
            .and_then(OrderedJson::as_array)
            .map(|entries| entries.len())
            .unwrap_or(0);
        println!(
            "{}@{}: {}, {} advisory(ies)",
            id.name,
            version,
            if yanked { "yanked" } else { "not yanked" },
            advisories
        );
    }
    Ok(0)
}

fn registry_for(options: &Options) -> Result<String, NopmError> {
    if let Some(registry) = &options.registry {
        return Ok(registry.clone());
    }
    if let Ok(project) = read_project(&options.directory, true)
        && let Some(registry) = project.registry
    {
        return Ok(registry);
    }
    Err(usage(
        "No registry is configured. Pass --registry <url> or set [registry].default.",
    ))
}

fn dependency_matches(dependency: &str, target: &str) -> bool {
    if dependency == target {
        return true;
    }
    parse_package_id(dependency)
        .map(|id| id.name == target)
        .unwrap_or(false)
}

fn emit_error(error: &NopmError, json: bool) -> i32 {
    if json {
        let response = ErrorResponse {
            ok: false,
            diagnostics: vec![DiagnosticResponse {
                code: error.code.clone(),
                message: error.message.clone(),
                path: error.path.clone(),
            }],
        };
        println!(
            "{}",
            serde_json::to_string_pretty(&response).unwrap_or_default()
        );
    } else {
        eprintln!("{}: {}", error.code, error.message);
    }
    if error.code == "NP0012" { 2 } else { 1 }
}

#[derive(Serialize)]
struct LockResponse<'a> {
    ok: bool,
    command: &'a str,
    lockfile: String,
    packages: usize,
    targets: Vec<String>,
}

#[derive(Serialize)]
struct AddResponse<'a> {
    ok: bool,
    action: &'a str,
    id: &'a str,
    packages: usize,
}

#[derive(Serialize)]
struct AddDryRunResponse<'a> {
    ok: bool,
    #[serde(rename = "dryRun")]
    dry_run: bool,
    action: &'a str,
    id: &'a str,
}

#[derive(Serialize)]
struct RemoveResponse<'a> {
    ok: bool,
    action: &'a str,
    name: &'a str,
}

#[derive(Serialize)]
struct RemoveDryRunResponse<'a> {
    ok: bool,
    #[serde(rename = "dryRun")]
    dry_run: bool,
    action: &'a str,
    name: &'a str,
}

#[derive(Serialize)]
struct SysInstallResponse<'a> {
    ok: bool,
    #[serde(rename = "dryRun", skip_serializing_if = "Option::is_none")]
    dry_run: Option<bool>,
    action: &'a str,
    tool: &'a str,
    manager: Option<&'a str>,
    commands: &'a [Vec<String>],
    installed: bool,
}

#[derive(Serialize)]
struct TreeResponse {
    ok: bool,
    command: &'static str,
    packages: Vec<TreePackage>,
}

#[derive(Serialize)]
struct TreePackage {
    id: String,
    name: String,
    source: String,
    version: Option<String>,
    resolved: Option<String>,
    registry: Option<String>,
    integrity: Option<String>,
    targets: Vec<String>,
    capabilities: Vec<String>,
    #[serde(rename = "lifecycleScripts")]
    lifecycle_scripts: bool,
    dependencies: Vec<String>,
    commit: Option<String>,
}

impl TreePackage {
    fn from_package(entry: &crate::resolve::LockPackage) -> Self {
        Self {
            id: entry.id.clone(),
            name: entry.name.clone(),
            source: entry.source.clone(),
            version: entry.version.clone(),
            resolved: entry.resolved.clone(),
            registry: entry.registry.clone(),
            integrity: entry.integrity.clone(),
            targets: entry.targets.clone(),
            capabilities: entry.capabilities.clone(),
            lifecycle_scripts: entry.lifecycle_scripts,
            dependencies: entry.dependencies.clone(),
            commit: entry.commit.clone(),
        }
    }
}

#[derive(Serialize)]
struct InstallResponse {
    ok: bool,
    command: &'static str,
    packages: usize,
    store: String,
}

#[derive(Serialize)]
struct WhyResponse<'a> {
    ok: bool,
    command: &'a str,
    target: &'a str,
    packages: Vec<String>,
}

#[derive(Serialize)]
struct DiagnosticResponse {
    code: String,
    message: String,
    path: Option<String>,
}

#[derive(Serialize)]
struct ErrorResponse {
    ok: bool,
    diagnostics: Vec<DiagnosticResponse>,
}
