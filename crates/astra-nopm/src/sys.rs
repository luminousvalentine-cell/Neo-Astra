use crate::command::CommandRunner;
use crate::error::NopmError;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SysContext {
    pub platform: String,
    pub uid: u32,
    pub managers: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SysToolPlan {
    pub tool: String,
    pub manager: Option<String>,
    pub commands: Vec<Vec<String>>,
    pub installed: bool,
}

const KNOWN_TOOLS: [&str; 3] = ["git", "cmake", "clang"];

struct ToolSpec {
    winget: &'static str,
    brew: &'static str,
    linux: &'static str,
}

fn tool_spec(tool: &str) -> Option<ToolSpec> {
    match tool {
        "git" => Some(ToolSpec {
            winget: "Git.Git",
            brew: "git",
            linux: "git",
        }),
        "cmake" => Some(ToolSpec {
            winget: "Kitware.CMake",
            brew: "cmake",
            linux: "cmake",
        }),
        "clang" => Some(ToolSpec {
            winget: "LLVM.LLVM",
            brew: "llvm",
            linux: "clang",
        }),
        _ => None,
    }
}

pub fn assert_known_sys_tool(tool: &str) -> Result<(), NopmError> {
    if tool_spec(tool).is_some() {
        return Ok(());
    }
    Err(NopmError::with_path(
        "NP0015",
        format!(
            "Unknown system tool '{tool}'. Known tools: {}.",
            KNOWN_TOOLS.join(", ")
        ),
        tool,
    ))
}

pub fn current_platform() -> String {
    match std::env::consts::OS {
        "windows" => "win32",
        "macos" => "darwin",
        other => other,
    }
    .to_owned()
}

#[cfg(unix)]
pub fn current_uid() -> u32 {
    unsafe { libc::getuid() }
}

#[cfg(not(unix))]
pub fn current_uid() -> u32 {
    0
}

pub fn detect_managers(platform: &str, runner: &dyn CommandRunner) -> Vec<String> {
    let mut managers = Vec::new();
    match platform {
        "win32" | "windows" => {
            if runner.run("winget", &["--version"], false).success() {
                managers.push("winget".to_owned());
            }
        }
        "darwin" | "macos" => {
            if runner.run("brew", &["--version"], false).success() {
                managers.push("brew".to_owned());
            }
        }
        _ => {
            for manager in ["apt-get", "dnf", "pacman", "zypper", "apk"] {
                let command = format!("command -v {manager}");
                if runner.run("sh", &["-c", &command], false).success() {
                    managers.push(manager.to_owned());
                }
            }
        }
    }
    managers
}

pub fn plan_sys_install(tool: &str, context: &SysContext) -> Result<SysToolPlan, NopmError> {
    assert_known_sys_tool(tool)?;
    let spec = tool_spec(tool).expect("known tool");

    if matches!(context.platform.as_str(), "win32" | "windows") {
        if !context.managers.iter().any(|manager| manager == "winget") {
            return Err(no_manager(tool));
        }
        return Ok(SysToolPlan {
            tool: tool.to_owned(),
            manager: Some("winget".to_owned()),
            commands: vec![vec![
                "winget".to_owned(),
                "install".to_owned(),
                "--id".to_owned(),
                spec.winget.to_owned(),
                "-e".to_owned(),
                "--accept-package-agreements".to_owned(),
                "--accept-source-agreements".to_owned(),
                "--silent".to_owned(),
            ]],
            installed: false,
        });
    }

    if matches!(context.platform.as_str(), "darwin" | "macos") {
        if context.uid == 0 {
            return Err(NopmError::with_path(
                "NP0017",
                "Homebrew refuses to run as root.",
                tool,
            ));
        }
        if !context.managers.iter().any(|manager| manager == "brew") {
            return Err(no_manager(tool));
        }
        return Ok(SysToolPlan {
            tool: tool.to_owned(),
            manager: Some("brew".to_owned()),
            commands: vec![vec![
                "brew".to_owned(),
                "install".to_owned(),
                spec.brew.to_owned(),
            ]],
            installed: false,
        });
    }

    let managers = ["apt-get", "dnf", "pacman", "zypper", "apk"];
    let Some(found) = managers
        .iter()
        .find(|manager| context.managers.iter().any(|present| present == **manager))
    else {
        return Err(no_manager(tool));
    };
    let package = spec.linux;
    let prefix: Vec<String> = if context.uid == 0 {
        Vec::new()
    } else {
        vec!["sudo".to_owned(), "-n".to_owned()]
    };
    let command = |parts: &[&str]| -> Vec<String> {
        let mut result = prefix.clone();
        result.extend(parts.iter().map(|part| (*part).to_owned()));
        result
    };
    let commands = match *found {
        "apt-get" => vec![
            command(&["apt-get", "update"]),
            command(&["apt-get", "install", "-y", package]),
        ],
        "dnf" => vec![command(&["dnf", "install", "-y", package])],
        "pacman" => vec![command(&["pacman", "-S", "--noconfirm", package])],
        "zypper" => vec![command(&[
            "zypper",
            "--non-interactive",
            "install",
            package,
        ])],
        "apk" => vec![command(&["apk", "add", package])],
        _ => return Err(no_manager(tool)),
    };
    Ok(SysToolPlan {
        tool: tool.to_owned(),
        manager: Some((*found).to_owned()),
        commands,
        installed: false,
    })
}

fn no_manager(tool: &str) -> NopmError {
    NopmError::with_path("NP0017", "No supported package manager found.", tool)
}

pub fn detect_installed(tool: &str, runner: &dyn CommandRunner) -> bool {
    runner.run(tool, &["--version"], false).success()
}

pub fn execute_sys_install(
    tool: &str,
    context: &SysContext,
    runner: &dyn CommandRunner,
) -> Result<SysToolPlan, NopmError> {
    assert_known_sys_tool(tool)?;
    if detect_installed(tool, runner) {
        return Ok(SysToolPlan {
            tool: tool.to_owned(),
            manager: None,
            commands: Vec::new(),
            installed: true,
        });
    }
    let plan = plan_sys_install(tool, context)?;
    for command in &plan.commands {
        let Some((binary, arguments)) = command.split_first() else {
            continue;
        };
        let args: Vec<&str> = arguments.iter().map(String::as_str).collect();
        let output = runner.run(binary, &args, true);
        if !output.success() {
            let command_line = command.join(" ");
            return Err(NopmError::with_path(
                "NP0018",
                format!("System install failed: {command_line}\n{}", output.stderr)
                    .trim()
                    .to_owned(),
                tool,
            ));
        }
    }
    Ok(plan)
}
