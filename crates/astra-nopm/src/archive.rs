use std::fs;
use std::io::Cursor;
use std::path::{Path, PathBuf};

use flate2::read::GzDecoder;

pub fn extract_archive(bytes: &[u8], destination: &Path) -> Result<(), String> {
    let tar_error = extract_tar(bytes, destination).err();
    if tar_error.is_none() {
        return Ok(());
    }

    let zip_error = extract_zip(bytes, destination).err();
    if zip_error.is_none() {
        return Ok(());
    }

    Err(zip_error.or(tar_error).unwrap_or_default())
}

fn extract_tar(bytes: &[u8], destination: &Path) -> Result<(), String> {
    if bytes.starts_with(&[0x1f, 0x8b]) {
        let decoder = GzDecoder::new(Cursor::new(bytes));
        let mut archive = tar::Archive::new(decoder);
        archive
            .unpack(destination)
            .map_err(|error| error.to_string())
    } else {
        let mut archive = tar::Archive::new(Cursor::new(bytes));
        archive
            .unpack(destination)
            .map_err(|error| error.to_string())
    }
}

fn extract_zip(bytes: &[u8], destination: &Path) -> Result<(), String> {
    let mut archive =
        zip::ZipArchive::new(Cursor::new(bytes)).map_err(|error| error.to_string())?;
    for index in 0..archive.len() {
        let mut entry = archive.by_index(index).map_err(|error| error.to_string())?;
        let Some(relative) = entry.enclosed_name() else {
            return Err("Zip archive contains an unsafe path.".to_owned());
        };
        let target = destination.join(relative);
        if entry.is_dir() {
            fs::create_dir_all(&target).map_err(|error| error.to_string())?;
            continue;
        }
        if let Some(parent) = target.parent() {
            fs::create_dir_all(parent).map_err(|error| error.to_string())?;
        }
        let mut output = fs::File::create(&target).map_err(|error| error.to_string())?;
        std::io::copy(&mut entry, &mut output).map_err(|error| error.to_string())?;
    }
    Ok(())
}

pub fn package_root(directory: &Path) -> PathBuf {
    let Ok(entries) = fs::read_dir(directory) else {
        return directory.to_path_buf();
    };
    let entries: Vec<_> = entries
        .filter_map(Result::ok)
        .filter(|entry| !entry.file_name().to_string_lossy().starts_with('.'))
        .collect();
    if entries.len() == 1 {
        let only = entries[0].path();
        if only.is_dir() {
            return only;
        }
    }
    directory.to_path_buf()
}

pub fn copy_directory(source: &Path, destination: &Path) -> Result<(), String> {
    fs::create_dir_all(destination).map_err(|error| error.to_string())?;
    for entry in fs::read_dir(source).map_err(|error| error.to_string())? {
        let entry = entry.map_err(|error| error.to_string())?;
        let file_type = entry.file_type().map_err(|error| error.to_string())?;
        let target = destination.join(entry.file_name());
        if file_type.is_dir() {
            copy_directory(&entry.path(), &target)?;
        } else if file_type.is_file() {
            fs::copy(entry.path(), &target).map_err(|error| error.to_string())?;
        }
    }
    Ok(())
}
