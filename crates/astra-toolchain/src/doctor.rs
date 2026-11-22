use serde::Serialize;
use std::fmt;
use std::io::Write;
use std::path::Path;
use std::process::{Command, Stdio};

use crate::install::{
    Downloader, Extractor, SystemDownloader, SystemExtractor, install_pinned_toolchain_default,
    install_pinned_toolchain_with, manual_instructions, pinned_for_host,
};
use crate::policy::ToolchainPolicy;

#[derive(Debug, Clone, Serialize)]
pub struct Attempt {
    pub rung: String,
    pub detail: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct Winner {
    pub rung: String,
    pub id: String,
    pub family: String,
    pub path: String,
    pub version: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct DoctorResult {
    pub attempts: Vec<Attempt>,
    pub winner: Option<Winner>,
    pub error: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ToolchainError {
    pub code: String,
    pub message: String,
}

impl ToolchainError {
    pub fn new(code: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            code: code.into(),
            message: message.into(),
        }
    }
}

impl fmt::Display for ToolchainError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{}: {}", self.code, self.message)
    }
}

impl std::error::Error for ToolchainError {}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProbeOutput {
    pub status: Option<i32>,
    pub stdout: String,
    pub stderr: String,
}

impl ProbeOutput {
    pub fn ok(&self) -> bool {
        self.status == Some(0)
    }
}

pub trait Probe {
    fn exists(&self, path: &str) -> bool;
    fn run(&self, bin: &str, args: &[&str], input: Option<&str>) -> ProbeOutput;
    fn read_file(&self, path: &str) -> Option<String>;
}

#[derive(Debug, Default, Clone, Copy)]
pub struct SystemProbe;

impl Probe for SystemProbe {
    fn exists(&self, path: &str) -> bool {
        Path::new(path).exists()
    }

    fn run(&self, bin: &str, args: &[&str], input: Option<&str>) -> ProbeOutput {
        let mut command = Command::new(bin);
        command.args(args);
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            command.creation_flags(0x0800_0000);
        }
        if let Some(input) = input {
            command
                .stdin(Stdio::piped())
                .stdout(Stdio::piped())
                .stderr(Stdio::piped());
            let Ok(mut child) = command.spawn() else {
                return ProbeOutput {
                    status: None,
                    stdout: String::new(),
                    stderr: format!("unable to launch {bin}"),
                };
            };
            if let Some(mut stdin) = child.stdin.take() {
                let _ = stdin.write_all(input.as_bytes());
            }
            return match child.wait_with_output() {
                Ok(output) => ProbeOutput {
                    status: output.status.code(),
                    stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
                    stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
                },
                Err(error) => ProbeOutput {
                    status: None,
                    stdout: String::new(),
                    stderr: error.to_string(),
                },
            };
        }
        match command.output() {
            Ok(output) => ProbeOutput {
                status: output.status.code(),
                stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
                stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
            },
            Err(error) => ProbeOutput {
                status: None,
                stdout: String::new(),
                stderr: error.to_string(),
            },
        }
    }

    fn read_file(&self, path: &str) -> Option<String> {
        std::fs::read_to_string(path).ok()
    }
}

pub fn family_from_binary(name: &str) -> String {
    let base = name
        .rsplit(['/', '\\'])
        .next()
        .unwrap_or(name)
        .trim_end_matches(".exe")
        .to_ascii_lowercase();
    match base.as_str() {
        "clang" => "clang".to_owned(),
        "gcc" | "cc" => "gcc".to_owned(),
        "zig" => "zig".to_owned(),
        _ => base,
    }
}

pub fn compiler_invocation(resolution: &Winner) -> (String, Vec<String>) {
    if resolution.family == "zig" {
        (resolution.path.clone(), vec!["cc".to_owned()])
    } else {
        (resolution.path.clone(), Vec::new())
    }
}

fn record(attempts: &mut Vec<Attempt>, rung: &str, detail: impl Into<String>) {
    attempts.push(Attempt {
        rung: rung.to_owned(),
        detail: detail.into(),
    });
}

fn find_on_path(name: &str, probe: &dyn Probe) -> Option<String> {
    probe
        .run(name, &["--version"], None)
        .ok()
        .then(|| name.to_owned())
}

fn version_of(probe: &dyn Probe, bin: &str) -> Option<String> {
    let output = probe.run(bin, &["--version"], None);
    if !output.ok() {
        return None;
    }
    let line = output.stdout.trim().lines().next().unwrap_or("").trim();
    if line.is_empty() {
        None
    } else {
        Some(line.to_owned())
    }
}

fn hosted_probe(probe: &dyn Probe, bin: &str) -> bool {
    probe
        .run(
            bin,
            &["-std=c17", "-fsyntax-only", "-x", "c", "-"],
            Some("#include <stdio.h>\nint main(void){return 0;}\n"),
        )
        .ok()
}

pub(crate) struct CacheResult {
    pub(crate) resolution: Option<Winner>,
    pub(crate) reason: String,
    pub(crate) can_fall_back: bool,
}

pub(crate) fn cache_dir_for(project_root: &Path, id: &str) -> String {
    format!("{}/.tools/toolchain/{id}", project_root.display())
}

pub(crate) fn lookup_cache(project_root: &Path, id: &str, probe: &dyn Probe) -> CacheResult {
    let at = id.find('@');
    let family = at.map_or(id, |index| &id[..index]);
    let versioned = at.is_some();
    let cache_dir = cache_dir_for(project_root, id);
    let marker_display = format!("{cache_dir}/toolchain.json");
    if !probe.exists(&marker_display) {
        return CacheResult {
            resolution: None,
            reason: format!("no cache marker at {marker_display}"),
            can_fall_back: !versioned,
        };
    }
    let Some(marker_text) = probe.read_file(&marker_display) else {
        return CacheResult {
            resolution: None,
            reason: format!("invalid cache marker at {marker_display}"),
            can_fall_back: false,
        };
    };
    let Ok(marker) = serde_json::from_str::<serde_json::Value>(&marker_text) else {
        return CacheResult {
            resolution: None,
            reason: format!("invalid cache marker at {marker_display}"),
            can_fall_back: false,
        };
    };
    let Some(marker) = marker.as_object() else {
        return CacheResult {
            resolution: None,
            reason: format!("invalid cache marker at {marker_display}"),
            can_fall_back: false,
        };
    };
    if marker.get("id").and_then(|value| value.as_str()) != Some(id) {
        return CacheResult {
            resolution: None,
            reason: format!(
                "cache marker id mismatch: expected {id}, got {}",
                marker
                    .get("id")
                    .map(ToString::to_string)
                    .unwrap_or_else(|| "undefined".to_owned())
            ),
            can_fall_back: false,
        };
    }
    if marker.get("family").and_then(|value| value.as_str()) != Some(family) {
        return CacheResult {
            resolution: None,
            reason: format!(
                "cache marker family mismatch: expected {family}, got {}",
                marker
                    .get("family")
                    .map(ToString::to_string)
                    .unwrap_or_else(|| "undefined".to_owned())
            ),
            can_fall_back: false,
        };
    }
    let Some(marker_version) = marker.get("version").and_then(|value| value.as_str()) else {
        return CacheResult {
            resolution: None,
            reason: format!("cache marker has no version at {marker_display}"),
            can_fall_back: false,
        };
    };
    if versioned && Some(marker_version) != at.map(|index| &id[index + 1..]) {
        return CacheResult {
            resolution: None,
            reason: format!(
                "cache marker version mismatch: expected {}, got {marker_version}",
                at.map(|index| &id[index + 1..]).unwrap_or("")
            ),
            can_fall_back: false,
        };
    }
    let candidates = [
        format!("{cache_dir}/bin/{family}"),
        format!("{cache_dir}/bin/{family}.exe"),
        format!("{cache_dir}/{family}"),
        format!("{cache_dir}/{family}.exe"),
    ];
    let Some(bin) = candidates
        .into_iter()
        .find(|candidate| probe.exists(candidate))
    else {
        return CacheResult {
            resolution: None,
            reason: format!("cache marker present but binary missing for {family} in {cache_dir}"),
            can_fall_back: false,
        };
    };
    CacheResult {
        resolution: Some(Winner {
            rung: "cache".to_owned(),
            id: id.to_owned(),
            family: family.to_owned(),
            path: bin,
            version: marker_version.to_owned(),
        }),
        reason: String::new(),
        can_fall_back: false,
    }
}

struct DownloadContext<'a> {
    downloader: &'a dyn Downloader,
    extractor: &'a dyn Extractor,
}

fn resolve_inner(
    project_root: &Path,
    policy: &ToolchainPolicy,
    env_cc: Option<&str>,
    probe: &dyn Probe,
    attempts: &mut Vec<Attempt>,
    download: Option<&DownloadContext<'_>>,
    explicit_no_download: bool,
) -> Result<Winner, ToolchainError> {
    if let Some(astra_cc) = env_cc.map(str::trim).filter(|value| !value.is_empty()) {
        record(attempts, "env", format!("ASTRA_CC={astra_cc}"));
        let path = if probe.exists(astra_cc) {
            Some(astra_cc.to_owned())
        } else {
            find_on_path(astra_cc, probe)
        };
        let Some(path) = path else {
            return Err(ToolchainError::new(
                "NS9103",
                format!("ASTRA_CC does not exist and is not on PATH: {astra_cc}"),
            ));
        };
        let Some(version) = version_of(probe, &path) else {
            return Err(ToolchainError::new(
                "NS9103",
                format!("ASTRA_CC does not answer --version: {astra_cc}"),
            ));
        };
        let family = family_from_binary(&path);
        return Ok(Winner {
            rung: "env".to_owned(),
            id: family.clone(),
            family,
            path,
            version,
        });
    }

    if policy.request != "auto" {
        let id = &policy.request;
        let cache_dir = cache_dir_for(project_root, id);
        let cache = lookup_cache(project_root, id, probe);
        if let Some(resolution) = cache.resolution {
            record(attempts, "cache", format!("resolved {}", resolution.path));
            return Ok(resolution);
        }
        record(attempts, "cache", cache.reason.clone());
        if !cache.can_fall_back {
            if id.contains('@') {
                let artifact = pinned_for_host(id);
                let marker = format!("{cache_dir}/toolchain.json");
                if probe.exists(&marker) {
                    let message = if explicit_no_download {
                        format!(
                            "{}; {}",
                            cache.reason,
                            manual_instructions(id, artifact, &cache_dir)
                        )
                    } else {
                        cache.reason.clone()
                    };
                    return Err(ToolchainError::new("NS9102", message));
                }
                let Some(download) = download else {
                    let message = if explicit_no_download {
                        format!(
                            "download disabled (--no-download); {}",
                            manual_instructions(id, artifact, &cache_dir)
                        )
                    } else {
                        cache.reason.clone()
                    };
                    return Err(ToolchainError::new("NS9102", message));
                };
                let Some(artifact) = artifact else {
                    return Err(ToolchainError::new(
                        "NS9102",
                        format!(
                            "no pinned artifact for {id} on this host; {}",
                            manual_instructions(id, None, &cache_dir)
                        ),
                    ));
                };
                let resolution = install_pinned_toolchain_with(
                    project_root,
                    id,
                    artifact,
                    probe,
                    download.downloader,
                    download.extractor,
                )?;
                record(attempts, "cache", format!("downloaded {id}"));
                return Ok(resolution);
            }
            return Err(ToolchainError::new("NS9102", cache.reason));
        }
        let family = id.as_str();
        if let Some(path) = find_on_path(family, probe) {
            if let Some(version) = version_of(probe, &path) {
                record(attempts, "path", format!("resolved {path}"));
                return Ok(Winner {
                    rung: "path".to_owned(),
                    id: family.to_owned(),
                    family: family.to_owned(),
                    path,
                    version,
                });
            }
            record(
                attempts,
                "path",
                format!("{family}: --version did not answer"),
            );
        } else {
            record(attempts, "path", format!("{family}: not on PATH"));
        }
        return Err(ToolchainError::new(
            "NS9102",
            format!("no cached or PATH toolchain for {family}"),
        ));
    }

    let candidates = ["clang", "gcc", "cc"];
    for candidate in candidates {
        let Some(bin) = find_on_path(candidate, probe) else {
            record(attempts, "path", format!("{candidate}: not on PATH"));
            continue;
        };
        let hosted = hosted_probe(probe, &bin);
        let version = version_of(probe, &bin);
        record(
            attempts,
            "path",
            format!(
                "{candidate}: hosted probe {}, --version {}",
                if hosted { "passed" } else { "failed" },
                if version.is_none() { "failed" } else { "ok" }
            ),
        );
        if hosted && let Some(version) = version {
            let family = family_from_binary(candidate);
            return Ok(Winner {
                rung: "path".to_owned(),
                id: family.clone(),
                family,
                path: bin,
                version,
            });
        }
    }
    for candidate in candidates {
        let Some(bin) = find_on_path(candidate, probe) else {
            continue;
        };
        let Some(version) = version_of(probe, &bin) else {
            continue;
        };
        record(
            attempts,
            "path",
            format!("{candidate}: fallback --version responder"),
        );
        let family = family_from_binary(candidate);
        return Ok(Winner {
            rung: "path".to_owned(),
            id: family.clone(),
            family,
            path: bin,
            version,
        });
    }
    Err(ToolchainError::new("NS9102", "no toolchain found on PATH"))
}

pub fn resolve(
    project_root: &Path,
    policy: &ToolchainPolicy,
    env_cc: Option<&str>,
    probe: &dyn Probe,
) -> Result<Winner, ToolchainError> {
    let mut attempts = Vec::new();
    resolve_inner(
        project_root,
        policy,
        env_cc,
        probe,
        &mut attempts,
        None,
        false,
    )
}

pub fn resolve_with_installer(
    project_root: &Path,
    policy: &ToolchainPolicy,
    env_cc: Option<&str>,
    probe: &dyn Probe,
    allow_download: bool,
    downloader: &dyn Downloader,
    extractor: &dyn Extractor,
) -> Result<Winner, ToolchainError> {
    let mut attempts = Vec::new();
    let context = DownloadContext {
        downloader,
        extractor,
    };
    resolve_inner(
        project_root,
        policy,
        env_cc,
        probe,
        &mut attempts,
        allow_download.then_some(&context),
        true,
    )
}

pub fn resolve_with_download(
    project_root: &Path,
    policy: &ToolchainPolicy,
    env_cc: Option<&str>,
    probe: &dyn Probe,
    allow_download: bool,
) -> Result<Winner, ToolchainError> {
    resolve_with_installer(
        project_root,
        policy,
        env_cc,
        probe,
        allow_download,
        &SystemDownloader,
        &SystemExtractor,
    )
}

pub fn resolve_with_attempts(
    project_root: &Path,
    policy: &ToolchainPolicy,
    env_cc: Option<&str>,
    probe: &dyn Probe,
) -> (Result<Winner, ToolchainError>, Vec<Attempt>) {
    let mut attempts = Vec::new();
    let result = resolve_inner(
        project_root,
        policy,
        env_cc,
        probe,
        &mut attempts,
        None,
        false,
    );
    (result, attempts)
}

fn doctor_from(result: Result<Winner, ToolchainError>, attempts: Vec<Attempt>) -> DoctorResult {
    let error = result
        .as_ref()
        .err()
        .map(|error| format!("{}: {}", error.code, error.message));
    let winner = result.ok();
    DoctorResult {
        attempts,
        winner,
        error,
    }
}

pub fn doctor(
    project_root: &Path,
    policy: &ToolchainPolicy,
    env_cc: Option<&str>,
    probe: &dyn Probe,
) -> DoctorResult {
    let (result, attempts) = resolve_with_attempts(project_root, policy, env_cc, probe);
    doctor_from(result, attempts)
}

pub fn doctor_with_installer(
    project_root: &Path,
    policy: &ToolchainPolicy,
    env_cc: Option<&str>,
    probe: &dyn Probe,
    allow_download: bool,
    downloader: &dyn Downloader,
    extractor: &dyn Extractor,
) -> DoctorResult {
    let mut base = doctor(project_root, policy, env_cc, probe);
    let versioned = policy.request != "auto"
        && policy.request.contains('@')
        && env_cc
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .is_none();
    if base.winner.is_some() {
        return base;
    }
    if !allow_download {
        if versioned {
            record(
                &mut base.attempts,
                "cache",
                "download disabled (--no-download)",
            );
        }
        return base;
    }
    if !versioned {
        return base;
    }
    match install_pinned_toolchain_default(
        project_root,
        &policy.request,
        probe,
        downloader,
        extractor,
    ) {
        Ok(winner) => {
            record(
                &mut base.attempts,
                "cache",
                format!("downloaded {}", policy.request),
            );
            base.winner = Some(winner);
            base.error = None;
            base
        }
        Err(error) => {
            let message = format!("{}: {}", error.code, error.message);
            record(
                &mut base.attempts,
                "cache",
                format!("download failed: {message}"),
            );
            base.error = Some(message);
            base
        }
    }
}

pub fn doctor_with_download(
    project_root: &Path,
    policy: &ToolchainPolicy,
    env_cc: Option<&str>,
    probe: &dyn Probe,
    allow_download: bool,
) -> DoctorResult {
    doctor_with_installer(
        project_root,
        policy,
        env_cc,
        probe,
        allow_download,
        &SystemDownloader,
        &SystemExtractor,
    )
}
