use std::fs;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::Serialize;

use crate::archive::{copy_directory, extract_archive, package_root};
use crate::error::NopmError;
use crate::http::HttpClient;
use crate::integrity::{sha256_hex, verify_integrity_bytes};
use crate::lockfile::{GENERATED_BY, Lockfile};
use crate::package_id::locale_cmp;
use crate::project::absolute_path;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StoreIndexEntry {
    pub id: String,
    pub source: String,
    pub root: String,
    pub integrity: Option<String>,
    pub targets: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StoreIndex {
    pub generated_by: String,
    pub packages: Vec<StoreIndexEntry>,
}

#[derive(Serialize)]
struct StoreEntryDocument<'a> {
    id: &'a str,
    source: &'a str,
    root: &'a str,
    integrity: Option<&'a str>,
    targets: &'a [String],
}

#[derive(Serialize)]
struct StoreIndexDocument<'a> {
    version: u8,
    #[serde(rename = "generatedBy")]
    generated_by: &'a str,
    packages: Vec<StoreEntryDocument<'a>>,
}

fn normalize_relative(project_root: &Path, path: &Path) -> String {
    path.strip_prefix(project_root)
        .map(|relative| relative.to_string_lossy().replace('\\', "/"))
        .unwrap_or_else(|_| path.to_string_lossy().replace('\\', "/"))
}

pub fn install_locked_packages(
    project_root: &Path,
    lockfile: &Lockfile,
    client: &dyn HttpClient,
) -> Result<StoreIndex, NopmError> {
    let store_directory = project_root.join(".astra").join("store");
    fs::create_dir_all(&store_directory).map_err(|_| {
        NopmError::with_path(
            "NP0011",
            "Unable to create the package store directory.",
            store_directory.to_string_lossy().into_owned(),
        )
    })?;

    let mut packages: Vec<&crate::resolve::LockPackage> = lockfile.packages.iter().collect();
    packages.sort_by(|left, right| locale_cmp(&left.id, &right.id));

    let mut entries = Vec::new();
    for entry in packages {
        entries.push(install_package(
            project_root,
            &store_directory,
            entry,
            client,
        )?);
    }

    let index = StoreIndex {
        generated_by: GENERATED_BY.to_owned(),
        packages: entries,
    };
    write_store_index(project_root, &index)?;
    Ok(index)
}

fn install_package(
    project_root: &Path,
    store_directory: &Path,
    entry: &crate::resolve::LockPackage,
    client: &dyn HttpClient,
) -> Result<StoreIndexEntry, NopmError> {
    if entry.source == "path" {
        let Some(resolved) = &entry.resolved else {
            return Err(NopmError::with_path(
                "NP0011",
                format!("Path package '{}' has no resolved path.", entry.id),
                entry.id.clone(),
            ));
        };
        let root = absolute_path(&project_root.join(resolved));
        if !root.exists() {
            return Err(NopmError::with_path(
                "NP0011",
                format!("Path package '{}' is missing.", entry.id),
                root.to_string_lossy().into_owned(),
            ));
        }
        return Ok(StoreIndexEntry {
            id: entry.id.clone(),
            source: entry.source.clone(),
            root: normalize_relative(project_root, &root),
            integrity: entry.integrity.clone(),
            targets: entry.targets.clone(),
        });
    }

    let key = store_key(entry);
    let store_path = store_directory.join(key);

    if !store_path.exists() {
        if entry.source == "git" {
            install_git_cache(project_root, entry, &store_path)?;
        } else {
            download_and_extract(entry, &store_path, client)?;
        }
    }

    Ok(StoreIndexEntry {
        id: entry.id.clone(),
        source: entry.source.clone(),
        root: normalize_relative(project_root, &store_path),
        integrity: entry.integrity.clone(),
        targets: entry.targets.clone(),
    })
}

fn install_git_cache(
    project_root: &Path,
    entry: &crate::resolve::LockPackage,
    store_path: &Path,
) -> Result<(), NopmError> {
    let Some(commit) = &entry.commit else {
        return Err(NopmError::with_path(
            "NP0011",
            format!("Git package '{}' has no pinned commit.", entry.id),
            entry.id.clone(),
        ));
    };
    let cache_path = project_root
        .join(".astra")
        .join("cache")
        .join("git")
        .join(commit);
    if !cache_path.exists() {
        return Err(NopmError::with_path(
            "NP0011",
            format!(
                "Git package '{}' is not in the local cache. Run nopm lock again.",
                entry.id
            ),
            cache_path.to_string_lossy().into_owned(),
        ));
    }
    copy_to_store(&cache_path, store_path)
}

fn download_and_extract(
    entry: &crate::resolve::LockPackage,
    store_path: &Path,
    client: &dyn HttpClient,
) -> Result<(), NopmError> {
    let (Some(resolved), Some(integrity)) = (&entry.resolved, &entry.integrity) else {
        return Err(NopmError::with_path(
            "NP0011",
            format!(
                "Package '{}' has no resolved URL or integrity hash.",
                entry.id
            ),
            entry.id.clone(),
        ));
    };

    let response = client.get(resolved, &[]).map_err(|message| {
        NopmError::with_path(
            "NP0011",
            format!("Unable to download '{}': {message}", entry.id),
            resolved.clone(),
        )
    })?;
    if !response.ok() {
        return Err(NopmError::with_path(
            "NP0011",
            format!(
                "Unable to download '{}': HTTP {}.",
                entry.id, response.status
            ),
            resolved.clone(),
        ));
    }

    verify_integrity_bytes(&response.body, integrity)?;

    let working = tempfile::tempdir().map_err(|error| {
        NopmError::with_path(
            "NP0011",
            format!("Unable to create a temporary directory: {error}"),
            resolved.clone(),
        )
    })?;
    let archive = working.path().join("package.tgz");
    let extracted = working.path().join("extracted");
    fs::create_dir_all(&extracted).map_err(|error| {
        NopmError::with_path(
            "NP0011",
            format!("Unable to create a temporary directory: {error}"),
            resolved.clone(),
        )
    })?;
    fs::write(&archive, &response.body).map_err(|error| {
        NopmError::with_path(
            "NP0011",
            format!("Unable to extract '{}': {error}", entry.id),
            resolved.clone(),
        )
    })?;
    extract_archive(&response.body, &extracted).map_err(|message| {
        NopmError::with_path(
            "NP0011",
            format!("Unable to extract '{}': {}", entry.id, message.trim()),
            resolved.clone(),
        )
    })?;
    let root = package_root(&extracted);
    copy_to_store(&root, store_path)
}

fn copy_to_store(source: &Path, destination: &Path) -> Result<(), NopmError> {
    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_nanos())
        .unwrap_or(0);
    let temporary = destination.with_file_name(format!(
        "{}.tmp-{}-{}",
        destination
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_else(|| "store".to_owned()),
        std::process::id(),
        unique
    ));
    let result = (|| -> Result<(), String> {
        fs::create_dir_all(&temporary).map_err(|error| error.to_string())?;
        copy_directory(source, &temporary)?;
        fs::rename(&temporary, destination).map_err(|error| error.to_string())
    })();
    if let Err(message) = result {
        let _ = fs::remove_dir_all(&temporary);
        return Err(NopmError::with_path(
            "NP0011",
            format!("Unable to copy package content to the store: {message}"),
            destination.to_string_lossy().into_owned(),
        ));
    }
    Ok(())
}

fn store_key(entry: &crate::resolve::LockPackage) -> String {
    let identity = format!(
        "{}\0{}\0{}",
        entry.id,
        entry.integrity.as_deref().unwrap_or(""),
        entry.commit.as_deref().unwrap_or("")
    );
    sha256_hex(identity.as_bytes())
}

pub fn store_index_path(project_root: &Path) -> PathBuf {
    project_root.join(".astra").join("store").join("index.json")
}

fn write_store_index(project_root: &Path, index: &StoreIndex) -> Result<(), NopmError> {
    let path = store_index_path(project_root);
    let document = StoreIndexDocument {
        version: 1,
        generated_by: &index.generated_by,
        packages: index
            .packages
            .iter()
            .map(|entry| StoreEntryDocument {
                id: &entry.id,
                source: &entry.source,
                root: &entry.root,
                integrity: entry.integrity.as_deref(),
                targets: &entry.targets,
            })
            .collect(),
    };
    let source = format!(
        "{}\n",
        serde_json::to_string_pretty(&document).unwrap_or_else(|_| "{}".to_owned())
    );

    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_nanos())
        .unwrap_or(0);
    let temporary =
        path.with_file_name(format!("index.json.tmp-{}-{}", std::process::id(), unique));
    let result = fs::write(&temporary, source).and_then(|_| fs::rename(&temporary, &path));
    if result.is_err() {
        let _ = fs::remove_file(&temporary);
        return Err(NopmError::with_path(
            "NP0011",
            "Unable to write the package store index.",
            path.to_string_lossy().into_owned(),
        ));
    }
    Ok(())
}
