use std::fs;
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::thread;
use std::time::Duration;

use astra_core::ast;
use astra_core::mode::Mode;
use astra_core::parser::parse;
use astra_nova::{NovaError, NovaOptions, Value as RuntimeValue};

use crate::watch::FileStamp;
use crate::{CliError, captured_call, emit_cli_error, print_error_text};

const POLL_INTERVAL: Duration = Duration::from_millis(300);
const IGNORED_DIRECTORIES: &[&str] = &[
    "target",
    "node_modules",
    "dist",
    "build",
    "out",
    "coverage",
    ".astra",
    ".git",
];

struct TestCase {
    name: String,
    parameters: usize,
}

pub(crate) fn run(path: &str, mode: Mode, infer_mode: bool, watch: bool) -> i32 {
    let root = Path::new(path);
    if let Err(error) = fs::metadata(root) {
        return emit_cli_error(&CliError::input(
            "NS9008",
            format!("Unable to access the test path: {error}"),
            "test",
            path.to_owned(),
            false,
        ));
    }

    let code = run_once(root, mode, infer_mode);
    if !watch {
        return code;
    }

    let mut previous = snapshot(root);
    loop {
        thread::sleep(POLL_INTERVAL);
        let current = snapshot(root);
        if current != previous {
            previous = current;
            run_once(root, mode, infer_mode);
        }
    }
}

fn run_once(root: &Path, mode: Mode, infer_mode: bool) -> i32 {
    let files = match collect_test_files(root) {
        Ok(files) => files,
        Err(error) => return emit_cli_error(&error),
    };

    if files.is_empty() {
        println!("No tests found.");
        return 0;
    }

    let mut passed = 0usize;
    let mut failed = 0usize;

    for path in files {
        let display = path.display().to_string();
        let source = match fs::read(&path) {
            Ok(bytes) => match String::from_utf8(bytes) {
                Ok(source) => source,
                Err(_) => {
                    eprintln!("{display}: error[NS9007]: Source is not valid UTF-8.");
                    failed += 1;
                    continue;
                }
            },
            Err(error) => {
                eprintln!("{display}: error[NS9008]: Unable to read the test file: {error}");
                failed += 1;
                continue;
            }
        };

        let module = match parse(&source) {
            Ok(module) => module,
            Err(diagnostics) => {
                print_error_text(&NovaError::Diagnostics(diagnostics), &display, &source);
                failed += 1;
                continue;
            }
        };

        let cases = test_cases(&module);
        if cases.is_empty() {
            println!("test {display} ... FAILED");
            eprintln!("  no @test functions or main function found");
            failed += 1;
            continue;
        }

        for case in cases {
            print!("test {display}::{} ... ", case.name);
            let _ = io::stdout().flush();

            if case.parameters != 0 {
                println!("FAILED");
                eprintln!("  test functions must not take parameters");
                failed += 1;
                continue;
            }

            let options = NovaOptions {
                mode,
                infer_mode,
                ..NovaOptions::default()
            };
            let (result, stdout, stderr) = captured_call(&source, options, Some(&case.name));
            let error = match result {
                Ok(RuntimeValue::Failure(error)) => Some(NovaError::UnhandledFailure(*error)),
                Ok(_) => None,
                Err(error) => Some(error),
            };

            println!("{}", if error.is_some() { "FAILED" } else { "ok" });
            if !stdout.is_empty() {
                print!("{stdout}");
            }
            if !stderr.is_empty() {
                eprint!("{stderr}");
            }

            if let Some(error) = error {
                print_error_text(&error, &display, &source);
                failed += 1;
            } else {
                passed += 1;
            }
        }
    }

    let total = passed + failed;
    if failed == 0 {
        println!("\ntest result: ok. {passed} passed; 0 failed; {total} total");
        0
    } else {
        println!("\ntest result: FAILED. {passed} passed; {failed} failed; {total} total");
        1
    }
}

fn collect_test_files(root: &Path) -> Result<Vec<PathBuf>, CliError> {
    let metadata = fs::metadata(root).map_err(|error| {
        CliError::input(
            "NS9008",
            format!("Unable to access the test path: {error}"),
            "test",
            root.display().to_string(),
            false,
        )
    })?;

    if metadata.is_file() {
        if root.extension().is_none_or(|extension| extension != "ns") {
            return Err(CliError::input(
                "NS9008",
                "The test target is not a .ns source file.",
                "test",
                root.display().to_string(),
                false,
            ));
        }
        return Ok(vec![root.to_path_buf()]);
    }

    if !metadata.is_dir() {
        return Err(CliError::input(
            "NS9008",
            "The test target is not a file or directory.",
            "test",
            root.display().to_string(),
            false,
        ));
    }

    let mut files = Vec::new();
    walk(root, &mut files)?;
    files.sort();
    Ok(files)
}

fn walk(directory: &Path, files: &mut Vec<PathBuf>) -> Result<(), CliError> {
    let entries = fs::read_dir(directory).map_err(|error| {
        CliError::input(
            "NS9008",
            format!("Unable to read the test directory: {error}"),
            "test",
            directory.display().to_string(),
            false,
        )
    })?;

    let mut entries = entries.collect::<Result<Vec<_>, _>>().map_err(|error| {
        CliError::input(
            "NS9008",
            format!("Unable to read the test directory: {error}"),
            "test",
            directory.display().to_string(),
            false,
        )
    })?;
    entries.sort_by_key(|entry| entry.file_name());

    for entry in entries {
        let path = entry.path();
        let Ok(file_type) = entry.file_type() else {
            continue;
        };

        if file_type.is_dir() {
            let name = entry.file_name();
            let name = name.to_string_lossy();
            if !name.starts_with('.') && !IGNORED_DIRECTORIES.contains(&name.as_ref()) {
                walk(&path, files)?;
            }
            continue;
        }

        if file_type.is_file() && is_test_file(&path) {
            files.push(path);
        }
    }

    Ok(())
}

fn is_test_file(path: &Path) -> bool {
    path.file_name()
        .and_then(|name| name.to_str())
        .is_some_and(|name| {
            let lower = name.to_ascii_lowercase();
            lower.ends_with(".test.ns")
        })
}

fn test_cases(module: &ast::Module) -> Vec<TestCase> {
    let mut cases = Vec::new();

    for item in &module.items {
        let ast::ItemKind::Function(function) = &item.kind else {
            continue;
        };
        if item
            .attributes
            .iter()
            .any(|attribute| attribute.name == "test")
        {
            cases.push(TestCase {
                name: function.name.clone(),
                parameters: function.parameters.len(),
            });
        }
    }

    if cases.is_empty()
        && module.items.iter().any(|item| {
            matches!(
                &item.kind,
                ast::ItemKind::Function(function) if function.name == "main"
            )
        })
    {
        cases.push(TestCase {
            name: "main".to_owned(),
            parameters: 0,
        });
    }

    cases
}

fn snapshot(root: &Path) -> Vec<(PathBuf, FileStamp)> {
    collect_test_files(root)
        .unwrap_or_default()
        .into_iter()
        .map(|path| (path.clone(), FileStamp::read(&path)))
        .collect()
}
