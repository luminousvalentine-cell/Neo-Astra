use std::fs;
use std::path::Path;
use std::thread;
use std::time::{Duration, SystemTime};

use astra_core::mode::Mode;

use crate::execute_file;

const POLL_INTERVAL: Duration = Duration::from_millis(250);

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct FileStamp {
    modified: Option<SystemTime>,
    len: u64,
}

impl FileStamp {
    pub(crate) fn read(path: &Path) -> Self {
        match fs::metadata(path) {
            Ok(metadata) => Self {
                modified: metadata.modified().ok(),
                len: metadata.len(),
            },
            Err(_) => Self {
                modified: None,
                len: 0,
            },
        }
    }
}

pub(crate) fn watch_file(file: &str, mode: Mode, infer_mode: bool, json: bool, once: bool) -> i32 {
    let path = Path::new(file);
    if !json {
        eprintln!("nova: watching {file}");
    }

    let mut previous = FileStamp::read(path);
    let code = execute_file(file, mode, infer_mode, json);
    if once {
        return code;
    }

    loop {
        let current = wait_for_change(path, &previous);
        previous = current;
        execute_file(file, mode, infer_mode, json);
    }
}

fn wait_for_change(path: &Path, previous: &FileStamp) -> FileStamp {
    loop {
        thread::sleep(POLL_INTERVAL);
        let current = FileStamp::read(path);
        if current != *previous {
            return current;
        }
    }
}
