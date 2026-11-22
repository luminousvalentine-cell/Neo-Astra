use std::process::{Command, Stdio};

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct CommandOutput {
    pub status: Option<i32>,
    pub stdout: String,
    pub stderr: String,
}

impl CommandOutput {
    pub fn success(&self) -> bool {
        self.status == Some(0)
    }
}

pub trait CommandRunner {
    fn run(&self, command: &str, args: &[&str], inherit: bool) -> CommandOutput;
}

#[derive(Clone, Copy, Debug, Default)]
pub struct SystemRunner;

impl CommandRunner for SystemRunner {
    fn run(&self, command: &str, args: &[&str], inherit: bool) -> CommandOutput {
        let mut process;
        #[cfg(windows)]
        {
            let is_script = command.ends_with(".cmd") || command.ends_with(".bat");
            if is_script {
                let shell = std::env::var("ComSpec").unwrap_or_else(|_| "cmd.exe".to_owned());
                process = Command::new(shell);
                process.args(["/d", "/s", "/c", command]);
                process.args(args);
            } else {
                process = Command::new(command);
                process.args(args);
            }
            use std::os::windows::process::CommandExt;
            process.creation_flags(0x0800_0000);
        }
        #[cfg(not(windows))]
        {
            process = Command::new(command);
            process.args(args);
        }

        if inherit {
            process
                .stdin(Stdio::inherit())
                .stdout(Stdio::inherit())
                .stderr(Stdio::inherit());
        } else {
            process
                .stdin(Stdio::null())
                .stdout(Stdio::piped())
                .stderr(Stdio::piped());
        }

        match process.output() {
            Ok(output) => CommandOutput {
                status: output.status.code(),
                stdout: if inherit {
                    String::new()
                } else {
                    String::from_utf8_lossy(&output.stdout).into_owned()
                },
                stderr: if inherit {
                    String::new()
                } else {
                    String::from_utf8_lossy(&output.stderr).into_owned()
                },
            },
            Err(error) => CommandOutput {
                status: None,
                stdout: String::new(),
                stderr: error.to_string(),
            },
        }
    }
}

pub fn command_name(name: &str) -> String {
    if cfg!(windows) && name == "npm" {
        "npm.cmd".to_owned()
    } else {
        name.to_owned()
    }
}
