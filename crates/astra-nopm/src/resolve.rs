use std::collections::{BTreeMap, BTreeSet, VecDeque};

use crate::adapters::{self, AdapterResolution};
use crate::command::CommandRunner;
use crate::error::NopmError;
use crate::http::HttpClient;
use crate::manifest::ManifestFile;
use crate::package_id::{
    DependencyRequest, PackageId, Source, format_package_id, locale_cmp, parse_dependency_request,
};
use crate::project::{ProjectManifest, TargetSpec, package_directory, read_project};

#[derive(Clone, Debug)]
pub struct Descriptor {
    pub id: PackageId,
    pub dependencies: Vec<(String, String)>,
    pub targets: Vec<String>,
    pub capabilities: Vec<String>,
    pub resolved: Option<String>,
    pub registry: Option<String>,
    pub integrity: Option<String>,
    pub commit: Option<String>,
    pub lifecycle_scripts: bool,
}

impl Descriptor {
    fn id_text(&self) -> String {
        format_package_id(&self.id)
    }
}

#[derive(Clone, Debug)]
pub struct LockPackage {
    pub id: String,
    pub name: String,
    pub source: String,
    pub version: Option<String>,
    pub resolved: Option<String>,
    pub registry: Option<String>,
    pub integrity: Option<String>,
    pub targets: Vec<String>,
    pub capabilities: Vec<String>,
    pub lifecycle_scripts: bool,
    pub dependencies: Vec<String>,
    pub commit: Option<String>,
}

#[derive(Clone, Debug)]
pub struct Resolution {
    pub targets: Vec<TargetSpec>,
    pub packages: Vec<LockPackage>,
}

#[derive(Clone, Debug)]
pub struct CollectedDescriptors {
    pub descriptors: Vec<Descriptor>,
    pub requests: Vec<DependencyRequest>,
}

pub fn collect_descriptors(
    project: &ProjectManifest,
    requests: Vec<DependencyRequest>,
    client: &dyn HttpClient,
    runner: &dyn CommandRunner,
) -> Result<CollectedDescriptors, NopmError> {
    let mut descriptors: BTreeMap<String, Descriptor> = BTreeMap::new();
    let mut exact_roots: Vec<DependencyRequest> = Vec::new();
    let mut queue: VecDeque<(DependencyRequest, bool)> = requests
        .into_iter()
        .map(|request| (request, true))
        .collect();

    while let Some((request, root)) = queue.pop_front() {
        let key = format_package_id(&request.id);
        if descriptors.contains_key(&key) {
            if root {
                exact_roots.push(request);
            }
            continue;
        }
        if project.package_sources.get(request.id.source.as_str()) == Some(&false) {
            return Err(NopmError::with_path(
                "NP0011",
                format!(
                    "The '{}' package source is disabled in the manifest.",
                    request.id.source.as_str()
                ),
                key,
            ));
        }

        let resolution = match request.id.source {
            Source::Path => {
                let descriptor = resolve_path_package(&project.directory, &request.id)?;
                let resolved_request =
                    parse_dependency_request(&request.alias, &format_package_id(&descriptor.id))?;
                AdapterResolution {
                    descriptor,
                    request: resolved_request,
                }
            }
            Source::Npm => adapters::npm::resolve_npm_package(&request.id, &request.alias, runner)?,
            Source::Git => adapters::git::resolve_git_package(
                &request.id,
                &request.alias,
                &project.directory.join(".astra").join("cache"),
                runner,
            )?,
            Source::Cargo => {
                adapters::cargo::resolve_cargo_package(&request.id, &request.alias, client, None)?
            }
            Source::Pypi => {
                adapters::pypi::resolve_pypi_package(&request.id, &request.alias, client, None)?
            }
            Source::Nuget => {
                adapters::nuget::resolve_nuget_package(&request.id, &request.alias, client, None)?
            }
            Source::Oci => adapters::oci::resolve_oci_package(&request.id, &request.alias, client)?,
            Source::Nova => {
                let Some(registry) = project.registry.as_deref() else {
                    return Err(NopmError::with_path(
                        "NP0011",
                        format!("No [registry].default is configured; cannot resolve '{key}'."),
                        key,
                    ));
                };
                adapters::registry::resolve_registry_package(
                    &request.id,
                    &request.alias,
                    client,
                    adapters::registry::RegistryOptions {
                        registry,
                        token: None,
                        require_signature: false,
                        public_key: None,
                    },
                )?
            }
        };
        let descriptor = resolution.descriptor;
        let resolved_request = resolution.request;

        let exact_key = descriptor.id_text();
        if let std::collections::btree_map::Entry::Vacant(entry) = descriptors.entry(exact_key) {
            for (alias, spec) in &descriptor.dependencies {
                queue.push_back((parse_dependency_request(alias, spec)?, false));
            }
            entry.insert(descriptor);
        }
        if root {
            exact_roots.push(resolved_request);
        }
    }

    let mut descriptors: Vec<Descriptor> = descriptors.into_values().collect();
    descriptors.sort_by(|left, right| locale_cmp(&left.id_text(), &right.id_text()));

    Ok(CollectedDescriptors {
        descriptors,
        requests: exact_roots,
    })
}

fn resolve_path_package(
    project_root: &std::path::Path,
    id: &PackageId,
) -> Result<Descriptor, NopmError> {
    let id_text = format_package_id(id);
    if id.source != Source::Path || id.path.is_none() {
        return Err(NopmError::with_path(
            "NP0011",
            "Expected a path package ID.",
            id_text,
        ));
    }
    let path = id.path.as_deref().unwrap_or("");
    let directory = package_directory(project_root, path)?;
    if !directory.exists() {
        return Err(NopmError::with_path(
            "NP0011",
            format!("Path package '{path}' does not exist."),
            path,
        ));
    }

    let project = read_project(&directory, false).map_err(|error| {
        NopmError::with_path(
            "NP0011",
            format!("Unable to read path package '{id_text}': {}", error.message),
            id_text.clone(),
        )
    })?;

    let mut descriptor_id = id.clone();
    if let Some(name) = &project.package_name {
        descriptor_id.name = name.clone();
    }

    Ok(Descriptor {
        id: descriptor_id,
        dependencies: project
            .dependencies
            .iter()
            .map(|dependency| (dependency.alias.clone(), dependency.request.spec.clone()))
            .collect(),
        targets: project
            .targets
            .iter()
            .map(|target| target.name.clone())
            .collect(),
        capabilities: project.capabilities.clone(),
        resolved: id.path.clone(),
        registry: None,
        integrity: None,
        commit: None,
        lifecycle_scripts: false,
    })
}

pub fn resolve_for_targets(
    requests: &[DependencyRequest],
    descriptors: &[Descriptor],
    targets: &[TargetSpec],
) -> Result<Resolution, NopmError> {
    validate_targets(targets)?;

    let mut sorted_targets: Vec<&TargetSpec> = targets.iter().collect();
    sorted_targets.sort_by(|left, right| locale_cmp(&left.name, &right.name));

    let mut packages: BTreeMap<String, (Descriptor, BTreeSet<String>)> = BTreeMap::new();
    for target in &sorted_targets {
        let graph = resolve_graph(requests, descriptors, target)?;
        for descriptor in graph {
            let key = descriptor.id_text();
            packages
                .entry(key)
                .or_insert_with(|| (descriptor, BTreeSet::new()))
                .1
                .insert(target.name.clone());
        }
    }

    let mut entries: Vec<(String, Descriptor, Vec<String>)> = packages
        .into_iter()
        .map(|(key, (descriptor, targets))| {
            let mut targets: Vec<String> = targets.into_iter().collect();
            targets.sort();
            (key, descriptor, targets)
        })
        .collect();
    entries.sort_by(|left, right| locale_cmp(&left.0, &right.0));

    let packages = entries
        .into_iter()
        .map(|(key, descriptor, targets)| lock_package(&key, &descriptor, targets))
        .collect();

    Ok(Resolution {
        targets: sorted_targets.into_iter().cloned().collect(),
        packages,
    })
}

fn validate_targets(targets: &[TargetSpec]) -> Result<(), NopmError> {
    if targets.is_empty() {
        return Err(NopmError::with_path(
            "NP0006",
            "At least one compiler target is required.",
            "targets",
        ));
    }
    let mut names = BTreeSet::new();
    for target in targets {
        if target.name.trim().is_empty() || target.name.trim() != target.name {
            return Err(NopmError::with_path(
                "NP0006",
                "Target names must be non-empty and trimmed.",
                target.name.clone(),
            ));
        }
        if target.backend.trim().is_empty() {
            return Err(NopmError::with_path(
                "NP0006",
                format!("Target '{}' requires a non-empty backend.", target.name),
                target.name.clone(),
            ));
        }
        if !crate::project::is_mode(&target.mode) {
            return Err(NopmError::with_path(
                "NP0007",
                format!(
                    "Target '{}' has unknown mode '{}'.",
                    target.name, target.mode
                ),
                target.mode.clone(),
            ));
        }
        if !names.insert(target.name.clone()) {
            return Err(NopmError::with_path(
                "NP0006",
                format!("Duplicate target '{}'.", target.name),
                target.name.clone(),
            ));
        }
    }
    Ok(())
}

fn resolve_graph(
    requests: &[DependencyRequest],
    descriptors: &[Descriptor],
    target: &TargetSpec,
) -> Result<Vec<Descriptor>, NopmError> {
    let mut descriptors_by_id: BTreeMap<String, Descriptor> = BTreeMap::new();
    for descriptor in descriptors {
        let key = descriptor.id_text();
        if descriptors_by_id.contains_key(&key) {
            return Err(NopmError::with_path(
                "NP0002",
                format!("Duplicate package descriptor '{key}'."),
                key,
            ));
        }
        descriptors_by_id.insert(key, descriptor.clone());
    }

    let mut root_map: BTreeMap<String, &DependencyRequest> = BTreeMap::new();
    for request in requests {
        if let Some(existing) = root_map.get(&request.alias)
            && format_package_id(&existing.id) != format_package_id(&request.id)
        {
            return Err(NopmError::with_path(
                "NP0005",
                format!(
                    "Dependency alias '{}' resolves to both '{}' and '{}'.",
                    request.alias,
                    format_package_id(&existing.id),
                    format_package_id(&request.id)
                ),
                request.alias.clone(),
            ));
        }
        root_map.insert(request.alias.clone(), request);
    }

    let mut sorted_roots: Vec<&DependencyRequest> = root_map.values().copied().collect();
    sorted_roots.sort_by(|left, right| {
        locale_cmp(&format_package_id(&left.id), &format_package_id(&right.id))
            .then_with(|| locale_cmp(&left.alias, &right.alias))
    });

    let mut visiting = BTreeSet::new();
    let mut stack: Vec<String> = Vec::new();
    let mut cache: BTreeMap<String, Descriptor> = BTreeMap::new();

    fn visit(
        id: &str,
        descriptors_by_id: &BTreeMap<String, Descriptor>,
        target: &TargetSpec,
        visiting: &mut BTreeSet<String>,
        stack: &mut Vec<String>,
        cache: &mut BTreeMap<String, Descriptor>,
    ) -> Result<(), NopmError> {
        if cache.contains_key(id) {
            return Ok(());
        }
        if visiting.contains(id) {
            let mut cycle = stack.clone();
            cycle.push(id.to_owned());
            return Err(NopmError::with_path(
                "NP0004",
                format!("Dependency cycle: {}.", cycle.join(" -> ")),
                id,
            ));
        }

        let descriptor = descriptors_by_id.get(id).ok_or_else(|| {
            NopmError::with_path(
                "NP0003",
                format!("No descriptor was supplied for '{id}'."),
                id,
            )
        })?;
        ensure_target_compatibility(descriptor, target)?;

        visiting.insert(id.to_owned());
        stack.push(id.to_owned());
        let mut dependencies: Vec<DependencyRequest> = descriptor
            .dependencies
            .iter()
            .map(|(alias, spec)| parse_dependency_request(alias, spec))
            .collect::<Result<_, _>>()?;
        dependencies.sort_by(|left, right| {
            locale_cmp(&format_package_id(&left.id), &format_package_id(&right.id))
                .then_with(|| locale_cmp(&left.alias, &right.alias))
        });
        for dependency in &dependencies {
            visit(
                &format_package_id(&dependency.id),
                descriptors_by_id,
                target,
                visiting,
                stack,
                cache,
            )?;
        }
        stack.pop();
        visiting.remove(id);

        cache.insert(id.to_owned(), descriptor.clone());
        Ok(())
    }

    for root in sorted_roots {
        visit(
            &format_package_id(&root.id),
            &descriptors_by_id,
            target,
            &mut visiting,
            &mut stack,
            &mut cache,
        )?;
    }

    let mut nodes: Vec<(String, Descriptor)> = cache.into_iter().collect();
    nodes.sort_by(|left, right| locale_cmp(&left.0, &right.0));
    Ok(nodes
        .into_iter()
        .map(|(_, descriptor)| descriptor)
        .collect())
}

fn ensure_target_compatibility(
    descriptor: &Descriptor,
    target: &TargetSpec,
) -> Result<(), NopmError> {
    if !descriptor.targets.is_empty() {
        let candidates: BTreeSet<String> = [
            Some(target.name.clone()),
            Some(target.backend.clone()),
            target.triple.clone(),
        ]
        .into_iter()
        .flatten()
        .map(|value| value.to_lowercase())
        .collect();
        if !descriptor
            .targets
            .iter()
            .any(|value| candidates.contains(&value.to_lowercase()))
        {
            let id = descriptor.id_text();
            return Err(NopmError::with_path(
                "NP0006",
                format!(
                    "Package '{}' does not support target '{}'.",
                    id, target.name
                ),
                id,
            ));
        }
    }

    let required = required_mode(descriptor)?;
    if let Some(required) = required
        && !mode_allows(&target.mode, required)
    {
        let id = descriptor.id_text();
        return Err(NopmError::with_path(
            "NP0007",
            format!(
                "Package '{}' requires '{}' mode, but target '{}' uses '{}'.",
                id, required, target.name, target.mode
            ),
            id,
        ));
    }
    Ok(())
}

fn required_mode(descriptor: &Descriptor) -> Result<Option<&'static str>, NopmError> {
    let mut required: Option<&'static str> = None;
    for capability in &descriptor.capabilities {
        let mode = match capability.as_str() {
            "web" => "web",
            "native" => "system",
            "wasm" => "core",
            _ => continue,
        };
        if mode == "web" && required == Some("system") {
            return Err(incompatible_capabilities(descriptor));
        }
        if mode == "system" && required == Some("web") {
            return Err(incompatible_capabilities(descriptor));
        }
        if required.is_none() || mode == "system" || (mode == "web" && required != Some("system")) {
            required = Some(mode);
        }
    }
    Ok(required)
}

fn incompatible_capabilities(descriptor: &Descriptor) -> NopmError {
    let id = descriptor.id_text();
    NopmError::with_path(
        "NP0007",
        format!(
            "Package '{}' requires both web and system capabilities, which no single mode allows.",
            id
        ),
        id,
    )
}

fn mode_allows(selected: &str, required: &str) -> bool {
    let allowed: &[&str] = match selected {
        "core" => &["core"],
        "app" => &["core", "app"],
        "web" => &["core", "app", "web"],
        "system" => &["core", "app", "system"],
        _ => &[],
    };
    allowed.contains(&required)
}

fn lock_package(id: &str, descriptor: &Descriptor, targets: Vec<String>) -> LockPackage {
    let mut dependencies: Vec<String> = descriptor
        .dependencies
        .iter()
        .filter_map(|(alias, spec)| {
            parse_dependency_request(alias, spec)
                .ok()
                .map(|request| format_package_id(&request.id))
        })
        .collect();
    dependencies.sort();

    let capabilities: BTreeSet<String> = descriptor.capabilities.iter().cloned().collect();

    LockPackage {
        id: id.to_owned(),
        name: descriptor.id.name.clone(),
        source: descriptor.id.source.as_str().to_owned(),
        version: descriptor.id.version.clone(),
        resolved: descriptor.resolved.clone(),
        registry: descriptor.registry.clone(),
        integrity: descriptor.integrity.clone(),
        targets,
        capabilities: capabilities.into_iter().collect(),
        lifecycle_scripts: descriptor.lifecycle_scripts,
        dependencies,
        commit: descriptor.commit.clone().or_else(|| {
            if descriptor.id.source == Source::Git {
                descriptor.id.reference.clone()
            } else {
                None
            }
        }),
    }
}

pub type CoreManifestFile = ManifestFile;
