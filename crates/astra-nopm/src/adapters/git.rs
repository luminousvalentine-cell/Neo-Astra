use std::fs;
use std::path::Path;

use crate::command::CommandRunner;
use crate::error::NopmError;
use crate::package_id::{PackageId, format_package_id, parse_dependency_request};
use crate::project::read_project;
use crate::resolve::Descriptor;

use super::AdapterResolution;

pub fn resolve_git_package(
    id: &PackageId,
    alias: &str,
    cache_root: &Path,
    runner: &dyn CommandRunner,
) -> Result<AdapterResolution, NopmError> {
    if id.source != crate::package_id::Source::Git || id.reference.is_none() {
        return Err(NopmError::with_path(
            "NP0011",
            "Expected a git package ID with a reference.",
            format_package_id(id),
        ));
    }
    let reference = id.reference.as_deref().unwrap_or("");
    let url = id.name.as_str();
    let commit = resolve_commit(url, reference, runner)?;
    let package_directory = cache_root.join("git").join(&commit);

    if !package_directory.exists() {
        clone(url, &commit, &package_directory, runner)?;
    }

    let project = read_project(&package_directory, false).map_err(|error| {
        NopmError::with_path(
            "NP0011",
            format!(
                "Unable to read git package '{}': {}",
                format_package_id(id),
                error.message
            ),
            format_package_id(id),
        )
    })?;

    let dependencies = project
        .dependencies
        .iter()
        .map(|dependency| (dependency.alias.clone(), dependency.request.spec.clone()))
        .collect();

    let descriptor = Descriptor {
        id: id.clone(),
        dependencies,
        targets: project
            .targets
            .iter()
            .map(|target| target.name.clone())
            .collect(),
        capabilities: project.capabilities.clone(),
        integrity: Some(format!("git:{commit}")),
        resolved: Some(url.to_owned()),
        registry: None,
        commit: Some(commit),
        lifecycle_scripts: false,
    };
    let request = parse_dependency_request(alias, &format_package_id(id))?;

    Ok(AdapterResolution {
        descriptor,
        request,
    })
}

fn resolve_commit(
    url: &str,
    reference: &str,
    runner: &dyn CommandRunner,
) -> Result<String, NopmError> {
    if is_commit(reference) {
        return Ok(reference.to_lowercase());
    }

    let result = runner.run("git", &["ls-remote", url, reference], false);
    if !result.success() {
        return Err(NopmError::new(
            "NP0011",
            format!(
                "Unable to resolve git reference '{reference}': {}",
                result.stderr.trim()
            ),
        ));
    }

    let lines: Vec<&str> = result
        .stdout
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .collect();
    if lines.is_empty() {
        return Err(NopmError::new(
            "NP0011",
            format!("Git reference '{reference}' was not found in '{url}'."),
        ));
    }

    let peeled = lines.iter().find(|line| line.ends_with("^{}"));
    let selected = peeled.copied().unwrap_or(lines[0]);
    let commit = selected.split_whitespace().next().unwrap_or("");
    if !is_commit(commit) {
        return Err(NopmError::new(
            "NP0011",
            format!("Git returned an invalid commit for '{reference}'."),
        ));
    }
    Ok(commit.to_lowercase())
}

fn is_commit(value: &str) -> bool {
    value.len() == 40 && value.chars().all(|character| character.is_ascii_hexdigit())
}

fn clone(
    url: &str,
    commit: &str,
    package_directory: &Path,
    runner: &dyn CommandRunner,
) -> Result<(), NopmError> {
    if let Some(parent) = package_directory.parent() {
        let _ = fs::create_dir_all(parent);
    }
    let clone_result = runner.run(
        "git",
        &["clone", "--quiet", url, path_text(package_directory)],
        false,
    );
    if !clone_result.success() {
        let _ = fs::remove_dir_all(package_directory);
        return Err(NopmError::new(
            "NP0011",
            format!("Unable to clone '{url}': {}", clone_result.stderr.trim()),
        ));
    }
    let checkout = runner.run(
        "git",
        &[
            "-C",
            path_text(package_directory),
            "checkout",
            "--quiet",
            "--detach",
            commit,
        ],
        false,
    );
    if !checkout.success() {
        let _ = fs::remove_dir_all(package_directory);
        return Err(NopmError::new(
            "NP0011",
            format!(
                "Unable to check out git commit '{commit}': {}",
                checkout.stderr.trim()
            ),
        ));
    }
    Ok(())
}

fn path_text(path: &Path) -> &str {
    path.to_str().unwrap_or("")
}
