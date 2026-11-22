use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use crate::doctor::{Probe, ToolchainError, Winner, cache_dir_for, lookup_cache};
use crate::pinned::{PinnedArtifact, find_pinned};

pub trait Downloader {
    fn download(&self, url: &str, destination: &Path) -> Result<(), String>;
}

pub trait Extractor {
    fn extract(&self, archive: &Path, destination: &Path) -> Result<(), String>;
}

#[derive(Debug, Default, Clone, Copy)]
pub struct SystemDownloader;

#[derive(Debug, Default, Clone, Copy)]
pub struct SystemExtractor;

impl Downloader for SystemDownloader {
    fn download(&self, url: &str, destination: &Path) -> Result<(), String> {
        let mut command = Command::new("curl");
        command
            .arg("-L")
            .arg("--fail")
            .arg("--silent")
            .arg("--show-error")
            .arg("--user-agent")
            .arg("neo-astra-toolchain/0.1.0")
            .arg("--output")
            .arg(destination)
            .arg(url);
        hide_window(&mut command);
        let output = command
            .output()
            .map_err(|error| format!("unable to launch curl: {error}"))?;
        if output.status.success() {
            return Ok(());
        }
        let stderr = String::from_utf8_lossy(&output.stderr);
        let stderr = stderr.trim();
        Err(if stderr.is_empty() {
            format!("curl exited with {}", output.status)
        } else {
            stderr.to_owned()
        })
    }
}

impl Extractor for SystemExtractor {
    fn extract(&self, archive: &Path, destination: &Path) -> Result<(), String> {
        let mut command = Command::new(system_tar());
        command.arg("-xf").arg(archive).arg("-C").arg(destination);
        hide_window(&mut command);
        let output = command
            .output()
            .map_err(|error| format!("unable to launch tar: {error}"))?;
        if output.status.success() {
            return Ok(());
        }
        let stderr = String::from_utf8_lossy(&output.stderr);
        let stderr = stderr.trim();
        Err(if stderr.is_empty() {
            format!("tar exited with {}", output.status)
        } else {
            stderr.to_owned()
        })
    }
}

fn hide_window(command: &mut Command) {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(0x0800_0000);
    }
    #[cfg(not(windows))]
    {
        let _ = command;
    }
}

fn system_tar() -> PathBuf {
    #[cfg(windows)]
    {
        let root = std::env::var_os("SystemRoot").unwrap_or_else(|| "C:\\Windows".into());
        let candidate = Path::new(&root).join("System32").join("tar.exe");
        if candidate.exists() {
            return candidate;
        }
    }
    PathBuf::from("tar")
}

pub fn host_platform() -> &'static str {
    match std::env::consts::OS {
        "windows" => "win32",
        "macos" => "darwin",
        other => other,
    }
}

pub fn host_arch() -> &'static str {
    match std::env::consts::ARCH {
        "x86_64" => "x64",
        "aarch64" => "arm64",
        other => other,
    }
}

pub fn pinned_for_host(id: &str) -> Option<&'static PinnedArtifact> {
    find_pinned(id, host_platform(), host_arch())
}

fn binary_name(artifact: &PinnedArtifact) -> String {
    if artifact.platform == "win32" {
        format!("{}.exe", artifact.family)
    } else {
        artifact.family.to_owned()
    }
}

fn quote_json(value: &str) -> String {
    serde_json::to_string(value).unwrap_or_else(|_| "\"\"".to_owned())
}

fn native_display(path: &Path) -> String {
    let display = path.display().to_string();
    #[cfg(windows)]
    {
        display.replace('/', "\\")
    }
    #[cfg(not(windows))]
    {
        display
    }
}
fn manual_artifact_instructions(artifact: &PinnedArtifact, cache_dir: &str) -> String {
    let cache = Path::new(cache_dir);
    let direct = native_display(&cache.join(artifact.family));
    let bin = native_display(&cache.join("bin").join(artifact.family));
    format!(
        "Manual install: download {}, verify sha256 {}, extract the {} binary to {} (or {}), and write {} with {{\"id\":{},\"family\":{},\"version\":{}}}. Use --no-download to skip network access and install manually.",
        artifact.url,
        artifact.sha256,
        artifact.family,
        direct,
        bin,
        native_display(&cache.join("toolchain.json")),
        quote_json(artifact.id),
        quote_json(artifact.family),
        quote_json(artifact.version),
    )
}

pub fn manual_instructions(id: &str, artifact: Option<&PinnedArtifact>, cache_dir: &str) -> String {
    if let Some(artifact) = artifact {
        return manual_artifact_instructions(artifact, cache_dir);
    }
    let at = id.find('@');
    let family = at.map_or(id, |index| &id[..index]);
    let version = at.map_or("unknown", |index| &id[index + 1..]);
    let cache = Path::new(cache_dir);
    format!(
        "Manual install: extract the {family} toolchain into {cache_dir} and write {} with {{\"id\":{},\"family\":{},\"version\":{}}}. Use --no-download to skip network access and install manually.",
        native_display(&cache.join("toolchain.json")),
        quote_json(id),
        quote_json(family),
        quote_json(version),
    )
}

fn unique_suffix() -> String {
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_nanos())
        .unwrap_or_default();
    let counter = COUNTER.fetch_add(1, Ordering::Relaxed);
    format!("{nanos}-{}-{counter}", std::process::id())
}

fn remove_dir(path: &Path) {
    let _ = fs::remove_dir_all(path);
}

fn install_marker(install_dir: &Path, id: &str, artifact: &PinnedArtifact) -> Result<(), String> {
    let marker = format!(
        "{{\n  \"id\": {},\n  \"family\": {},\n  \"version\": {},\n  \"url\": {},\n  \"sha256\": {}\n}}\n",
        quote_json(id),
        quote_json(artifact.family),
        quote_json(artifact.version),
        quote_json(artifact.url),
        quote_json(artifact.sha256),
    );
    fs::write(install_dir.join("toolchain.json"), marker)
        .map_err(|error| format!("unable to write the toolchain marker: {error}"))
}

pub fn install_pinned_toolchain_with(
    project_root: &Path,
    id: &str,
    artifact: &PinnedArtifact,
    probe: &dyn Probe,
    downloader: &dyn Downloader,
    extractor: &dyn Extractor,
) -> Result<Winner, ToolchainError> {
    let cache_dir = PathBuf::from(cache_dir_for(project_root, id));
    let marker = cache_dir.join("toolchain.json");
    let cached = lookup_cache(project_root, id, probe);
    if let Some(resolution) = cached.resolution {
        return Ok(resolution);
    }
    if marker.exists() || probe.exists(&marker.to_string_lossy()) {
        return Err(ToolchainError::new(
            "NS9102",
            format!(
                "{}; {}",
                cached.reason,
                manual_artifact_instructions(artifact, &cache_dir.to_string_lossy())
            ),
        ));
    }

    let tools_dir = project_root.join(".tools").join("toolchain");
    fs::create_dir_all(&tools_dir).map_err(|error| {
        ToolchainError::new(
            "NS9102",
            format!(
                "install failed: {error}; {}",
                manual_artifact_instructions(artifact, &cache_dir.to_string_lossy())
            ),
        )
    })?;

    let suffix = unique_suffix();
    let staging = tools_dir.join(format!(".staging-{suffix}"));
    let extracted = staging.join("extracted");
    let install_dir = staging.join("install");
    let backup_dir = tools_dir.join(format!(".backup-{suffix}"));
    let archive = staging.join("archive");
    let mut moved_old_cache = false;

    let result = (|| -> Result<Winner, String> {
        fs::create_dir_all(&extracted)
            .map_err(|error| format!("unable to create the staging directory: {error}"))?;
        fs::create_dir_all(&install_dir)
            .map_err(|error| format!("unable to create the install directory: {error}"))?;

        downloader
            .download(artifact.url, &archive)
            .map_err(|error| {
                format!(
                    "download failed for {}: {error}; {}",
                    artifact.url,
                    manual_artifact_instructions(artifact, &cache_dir.to_string_lossy())
                )
            })?;

        let metadata = fs::metadata(&archive).map_err(|error| {
            format!(
                "download failed for {}: {error}; {}",
                artifact.url,
                manual_artifact_instructions(artifact, &cache_dir.to_string_lossy())
            )
        })?;
        if metadata.len() != artifact.size {
            return Err(format!(
                "archive size mismatch: expected {} bytes, received {}; {}",
                artifact.size,
                metadata.len(),
                manual_artifact_instructions(artifact, &cache_dir.to_string_lossy())
            ));
        }
        let bytes = fs::read(&archive).map_err(|error| {
            format!(
                "unable to read the downloaded archive: {error}; {}",
                manual_artifact_instructions(artifact, &cache_dir.to_string_lossy())
            )
        })?;
        let digest = sha256_hex(&bytes);
        if digest != artifact.sha256 {
            return Err(format!(
                "archive sha256 mismatch: expected {}, received {digest}; {}",
                artifact.sha256,
                manual_artifact_instructions(artifact, &cache_dir.to_string_lossy())
            ));
        }

        extractor.extract(&archive, &extracted).map_err(|error| {
            format!(
                "archive extraction failed: {error}; {}",
                manual_artifact_instructions(artifact, &cache_dir.to_string_lossy())
            )
        })?;

        let entries: Vec<_> = fs::read_dir(&extracted)
            .map_err(|error| format!("unable to inspect the extracted archive: {error}"))?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|error| format!("unable to inspect the extracted archive: {error}"))?;
        if entries.is_empty() {
            return Err(format!(
                "the downloaded archive was empty; {}",
                manual_artifact_instructions(artifact, &cache_dir.to_string_lossy())
            ));
        }
        let source_root = if entries.len() == 1 && entries[0].path().is_dir() {
            entries[0].path()
        } else {
            extracted.clone()
        };
        for entry in fs::read_dir(&source_root)
            .map_err(|error| format!("unable to inspect the extracted archive: {error}"))?
        {
            let entry =
                entry.map_err(|error| format!("unable to read an archive entry: {error}"))?;
            let target = install_dir.join(entry.file_name());
            fs::rename(entry.path(), target)
                .map_err(|error| format!("unable to stage an archive entry: {error}"))?;
        }

        let binary = binary_name(artifact);
        let candidates = [
            install_dir.join(&binary),
            install_dir.join("bin").join(&binary),
        ];
        if !candidates.iter().any(|candidate| candidate.exists()) {
            return Err(format!(
                "the archive did not contain {binary}; {}",
                manual_artifact_instructions(artifact, &cache_dir.to_string_lossy())
            ));
        }

        install_marker(&install_dir, id, artifact)?;

        if cache_dir.exists() {
            fs::rename(&cache_dir, &backup_dir)
                .map_err(|error| format!("unable to replace the existing cache: {error}"))?;
            moved_old_cache = true;
        }
        match fs::rename(&install_dir, &cache_dir) {
            Ok(()) => {}
            Err(error) => {
                let concurrent = lookup_cache(project_root, id, probe);
                if let Some(resolution) = concurrent.resolution {
                    remove_dir(&staging);
                    if moved_old_cache {
                        remove_dir(&backup_dir);
                    }
                    return Ok(resolution);
                }
                return Err(format!("install failed: {error}"));
            }
        }
        if moved_old_cache {
            remove_dir(&backup_dir);
        }
        remove_dir(&staging);

        let installed = lookup_cache(project_root, id, probe);
        installed.resolution.ok_or_else(|| {
            format!(
                "installed cache could not be verified: {}; {}",
                installed.reason,
                manual_artifact_instructions(artifact, &cache_dir.to_string_lossy())
            )
        })
    })();

    match result {
        Ok(resolution) => Ok(resolution),
        Err(message) => {
            remove_dir(&staging);
            if moved_old_cache && backup_dir.exists() && !cache_dir.exists() {
                let _ = fs::rename(&backup_dir, &cache_dir);
            }
            Err(ToolchainError::new("NS9102", message))
        }
    }
}

pub fn install_pinned_toolchain(
    project_root: &Path,
    id: &str,
    probe: &dyn Probe,
) -> Result<Winner, ToolchainError> {
    install_pinned_toolchain_default(project_root, id, probe, &SystemDownloader, &SystemExtractor)
}

pub fn install_pinned_toolchain_default(
    project_root: &Path,
    id: &str,
    probe: &dyn Probe,
    downloader: &dyn Downloader,
    extractor: &dyn Extractor,
) -> Result<Winner, ToolchainError> {
    let cache_dir = cache_dir_for(project_root, id);
    let Some(artifact) = pinned_for_host(id) else {
        return Err(ToolchainError::new(
            "NS9102",
            format!(
                "No pinned artifact for {id} on this host. {}",
                manual_instructions(id, None, &cache_dir)
            ),
        ));
    };
    install_pinned_toolchain_with(project_root, id, artifact, probe, downloader, extractor)
}

const SHA256_K: [u32; 64] = [
    0x428a2f98, 0x71374491, 0xb5c0fbcf, 0xe9b5dba5, 0x3956c25b, 0x59f111f1, 0x923f82a4, 0xab1c5ed5,
    0xd807aa98, 0x12835b01, 0x243185be, 0x550c7dc3, 0x72be5d74, 0x80deb1fe, 0x9bdc06a7, 0xc19bf174,
    0xe49b69c1, 0xefbe4786, 0x0fc19dc6, 0x240ca1cc, 0x2de92c6f, 0x4a7484aa, 0x5cb0a9dc, 0x76f988da,
    0x983e5152, 0xa831c66d, 0xb00327c8, 0xbf597fc7, 0xc6e00bf3, 0xd5a79147, 0x06ca6351, 0x14292967,
    0x27b70a85, 0x2e1b2138, 0x4d2c6dfc, 0x53380d13, 0x650a7354, 0x766a0abb, 0x81c2c92e, 0x92722c85,
    0xa2bfe8a1, 0xa81a664b, 0xc24b8b70, 0xc76c51a3, 0xd192e819, 0xd6990624, 0xf40e3585, 0x106aa070,
    0x19a4c116, 0x1e376c08, 0x2748774c, 0x34b0bcb5, 0x391c0cb3, 0x4ed8aa4a, 0x5b9cca4f, 0x682e6ff3,
    0x748f82ee, 0x78a5636f, 0x84c87814, 0x8cc70208, 0x90befffa, 0xa4506ceb, 0xbef9a3f7, 0xc67178f2,
];

pub fn sha256_hex(bytes: &[u8]) -> String {
    let mut state: [u32; 8] = [
        0x6a09e667, 0xbb67ae85, 0x3c6ef372, 0xa54ff53a, 0x510e527f, 0x9b05688c, 0x1f83d9ab,
        0x5be0cd19,
    ];
    let bit_len = (bytes.len() as u64).wrapping_mul(8);
    let mut padded = Vec::with_capacity(bytes.len() + 72);
    padded.extend_from_slice(bytes);
    padded.push(0x80);
    while padded.len() % 64 != 56 {
        padded.push(0);
    }
    padded.extend_from_slice(&bit_len.to_be_bytes());

    for chunk in padded.as_chunks::<64>().0 {
        let mut w = [0u32; 64];
        for (index, word) in chunk.as_chunks::<4>().0.iter().enumerate() {
            w[index] = u32::from_be_bytes(*word);
        }
        for index in 16..64 {
            let s0 = w[index - 15].rotate_right(7)
                ^ w[index - 15].rotate_right(18)
                ^ (w[index - 15] >> 3);
            let s1 = w[index - 2].rotate_right(17)
                ^ w[index - 2].rotate_right(19)
                ^ (w[index - 2] >> 10);
            w[index] = w[index - 16]
                .wrapping_add(s0)
                .wrapping_add(w[index - 7])
                .wrapping_add(s1);
        }
        let mut a = state[0];
        let mut b = state[1];
        let mut c = state[2];
        let mut d = state[3];
        let mut e = state[4];
        let mut f = state[5];
        let mut g = state[6];
        let mut h = state[7];
        for index in 0..64 {
            let s1 = e.rotate_right(6) ^ e.rotate_right(11) ^ e.rotate_right(25);
            let ch = (e & f) ^ ((!e) & g);
            let temp1 = h
                .wrapping_add(s1)
                .wrapping_add(ch)
                .wrapping_add(SHA256_K[index])
                .wrapping_add(w[index]);
            let s0 = a.rotate_right(2) ^ a.rotate_right(13) ^ a.rotate_right(22);
            let maj = (a & b) ^ (a & c) ^ (b & c);
            let temp2 = s0.wrapping_add(maj);
            h = g;
            g = f;
            f = e;
            e = d.wrapping_add(temp1);
            d = c;
            c = b;
            b = a;
            a = temp1.wrapping_add(temp2);
        }
        state[0] = state[0].wrapping_add(a);
        state[1] = state[1].wrapping_add(b);
        state[2] = state[2].wrapping_add(c);
        state[3] = state[3].wrapping_add(d);
        state[4] = state[4].wrapping_add(e);
        state[5] = state[5].wrapping_add(f);
        state[6] = state[6].wrapping_add(g);
        state[7] = state[7].wrapping_add(h);
    }
    state
        .iter()
        .map(|word| format!("{word:08x}"))
        .collect::<String>()
}
