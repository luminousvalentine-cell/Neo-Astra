use std::collections::{BTreeMap, BTreeSet};
use std::env;
use std::fmt;
use std::fs;
use std::path::{Path, PathBuf};
use std::rc::Rc;

use astra_core::ast;
use astra_core::checker::{CheckOptions, CheckResult, check_resolved};
use astra_core::diagnostic::Diagnostic;
use astra_core::hir;
use astra_core::mode::Mode;
use astra_core::parser::parse;
use astra_core::resolver::{ModuleGraph, ModuleResolver, collect_module_specifiers};
use astra_core::span::Span;
use astra_core::type_surface::{ModuleTypeSurface, module_type_surface_with_surfaces};

use crate::error::NovaError;
use crate::host::{Host, StdHost};
use crate::interpreter::{Interpreter, NovaOptions};
use crate::value::Value;
use astra_capabilities::{infer_module_mode_named, infer_project};

use Diagnostic as CoreDiagnostic;

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ModuleId(String);

impl ModuleId {

    pub fn parse(value: impl Into<String>) -> Result<Self, ProjectError> {
        let value = value.into();
        let id = Self(value.clone());
        if let Some(message) = id.validation_error() {
            return Err(ProjectError::InvalidModule { id: value, message });
        }
        Ok(id)
    }

    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }

    #[must_use]
    pub fn parent(&self) -> &str {
        self.0.rsplit_once('/').map_or("", |(parent, _)| parent)
    }

    #[must_use]
    pub fn stem(&self) -> &str {
        let name = self.0.rsplit('/').next().unwrap_or(&self.0);
        name.strip_suffix(".ns").unwrap_or(name)
    }

    fn validation_error(&self) -> Option<String> {
        let value = &self.0;
        if value.is_empty() {
            return Some("module identity cannot be empty".to_owned());
        }
        if value.contains('\\') {
            return Some("module identity must use forward slashes".to_owned());
        }
        if value.starts_with('/') || value.contains(':') {
            return Some("module identity must be project-relative".to_owned());
        }
        if !value.ends_with(".ns") {
            return Some("module identity must end in '.ns'".to_owned());
        }
        for component in value.split('/') {
            if component.is_empty() || component == "." || component == ".." {
                return Some("module identity contains an invalid path component".to_owned());
            }
        }
        None
    }
}

impl fmt::Display for ModuleId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProjectDiagnostic {

    pub module: ModuleId,

    pub path: PathBuf,

    pub diagnostic: Diagnostic,
}

#[derive(Debug, Clone)]
pub struct Project {

    pub root: PathBuf,

    pub entry: ModuleId,

    pub mode: Mode,

    pub modules: BTreeMap<ModuleId, CompiledModule>,
}

#[derive(Debug, Clone)]
pub struct CompiledModule {
    pub id: ModuleId,
    pub path: PathBuf,
    pub text: Rc<str>,
    pub ast: ast::Module,
    pub resolved: hir::Module,
    pub checked: CheckResult,
}

#[derive(Debug, Clone)]
pub struct ProjectOptions {
    pub root: PathBuf,
    pub entry: PathBuf,
    pub mode: Mode,
    pub infer_mode: bool,
    pub allow_root_escape: bool,
}

#[derive(Debug, Clone)]
pub enum ProjectError {

    Diagnostics(Vec<ProjectDiagnostic>),

    Runtime {
        module: ModuleId,
        path: PathBuf,
        error: Box<NovaError>,
    },

    InvalidModule { id: String, message: String },
}

impl ProjectError {

    #[must_use]
    pub fn diagnostics(&self) -> &[ProjectDiagnostic] {
        match self {
            Self::Diagnostics(diagnostics) => diagnostics,
            Self::Runtime { .. } | Self::InvalidModule { .. } => &[],
        }
    }

    #[must_use]
    pub fn code(&self) -> Option<&'static str> {
        match self {
            Self::Diagnostics(_) => None,
            Self::Runtime { error, .. } => Some(error.code()),
            Self::InvalidModule { .. } => Some("NS3012"),
        }
    }
}

impl fmt::Display for ProjectError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Diagnostics(diagnostics) => {
                formatter.write_str("the project did not pass the frontend.")?;
                for diagnostic in diagnostics {
                    write!(
                        formatter,
                        "\n{}: [{}{}]: {}",
                        diagnostic.path.display(),
                        diagnostic.diagnostic.severity.as_str(),
                        diagnostic.diagnostic.code,
                        diagnostic.diagnostic.message
                    )?;
                }
                Ok(())
            }
            Self::Runtime {
                module,
                path,
                error,
            } => write!(formatter, "{} ({module}): {error}", path.display()),
            Self::InvalidModule { id, message } => {
                write!(formatter, "invalid module id '{id}': {message}")
            }
        }
    }
}

impl std::error::Error for ProjectError {}

pub fn load_project(options: ProjectOptions) -> Result<Project, ProjectError> {
    let root = fs::canonicalize(&options.root).map_err(|error| {
        io_error(
            None,
            &options.root,
            "NS9008",
            format!("Unable to access the project root: {error}"),
        )
    })?;
    let entry_path = fs::canonicalize(&options.entry).map_err(|error| {
        io_error(
            None,
            &options.entry,
            "NS9008",
            format!("Unable to access the entry module: {error}"),
        )
    })?;
    let Ok(relative) = entry_path.strip_prefix(&root) else {
        if !options.allow_root_escape {
            return Err(io_error(
                None,
                &options.entry,
                "NS9008",
                "The entry module is outside the project root.".to_owned(),
            ));
        }
        return Err(ProjectError::InvalidModule {
            id: entry_path.display().to_string(),
            message: "entry files outside the root cannot be given a canonical module id"
                .to_owned(),
        });
    };
    let entry = module_id_from_path(relative).map_err(|message| ProjectError::InvalidModule {
        id: relative.display().to_string(),
        message,
    })?;

    let mut sources = SourceSet::from_disk(root.clone());
    sources.ensure(&entry, None)?;
    assemble(entry, options.mode, options.infer_mode, sources)
}

pub fn project_from_sources(
    entry: ModuleId,
    sources: BTreeMap<ModuleId, String>,
    options: NovaOptions,
) -> Result<Project, ProjectError> {
    let mut source_set = SourceSet::from_memory(PathBuf::new());
    for (id, text) in sources {
        if let Some(message) = id.validation_error() {
            return Err(ProjectError::InvalidModule {
                id: id.to_string(),
                message,
            });
        }
        if let Some(previous) = source_set
            .case_ids
            .insert(id.as_str().to_lowercase(), id.clone())
            && previous != id
        {
            return Err(ProjectError::Diagnostics(vec![ProjectDiagnostic {
                module: previous.clone(),
                path: PathBuf::from(previous.as_str()),
                diagnostic: CoreDiagnostic::error(
                    "NS3012",
                    format!(
                        "Module ids '{}' and '{}' collide case-insensitively.",
                        previous.as_str(),
                        id.as_str()
                    ),
                ),
            }]));
        }
        source_set
            .files
            .insert(id.clone(), (PathBuf::from(id.as_str()), text));
    }
    assemble(entry, options.mode, options.infer_mode, source_set)
}

pub fn build_project_interpreter(
    project: Project,
    options: NovaOptions,
    host: Box<dyn Host>,
) -> Result<Interpreter, ProjectError> {
    let entry = project.entry.clone();
    for (id, module) in &project.modules {
        if let Some(span) = module.checked.manual_memory_scopes.first() {
            return Err(ProjectError::Runtime {
                module: id.clone(),
                path: module.path.clone(),
                error: Box::new(NovaError::ManualMemoryUnsupported { span: *span }),
            });
        }
    }
    let entry_path = project
        .modules
        .get(&entry)
        .map_or_else(PathBuf::new, |module| module.path.clone());
    let checked = project
        .modules
        .into_iter()
        .map(|(id, module)| (id.to_string(), module.checked))
        .collect();
    Interpreter::from_modules(entry.to_string(), checked, options, host).map_err(|error| {
        ProjectError::Runtime {
            module: entry,
            path: entry_path,
            error: Box::new(error),
        }
    })
}

pub fn run_project(entry: impl AsRef<Path>, options: NovaOptions) -> Result<Value, ProjectError> {
    let entry = entry.as_ref();
    let root = project_root(entry);
    let project = load_project(ProjectOptions {
        root,
        entry: entry.to_path_buf(),
        mode: options.mode,
        infer_mode: options.infer_mode,
        allow_root_escape: false,
    })?;
    let entry_id = project.entry.clone();
    let module_paths = project
        .modules
        .iter()
        .map(|(id, module)| (id.to_string(), module.path.clone()))
        .collect::<BTreeMap<_, _>>();
    let fallback_path = module_paths
        .get(entry_id.as_str())
        .cloned()
        .unwrap_or_else(|| entry.to_path_buf());
    let mut interpreter = build_project_interpreter(project, options, Box::new(StdHost))?;
    match interpreter.run() {
        Ok(value) => Ok(value),
        Err(error) => {
            let module = interpreter
                .fault_module()
                .and_then(|id| ModuleId::parse(id).ok())
                .unwrap_or_else(|| module_for_error(&interpreter, &entry_id));
            let path = module_paths
                .get(module.as_str())
                .cloned()
                .unwrap_or(fallback_path);
            Err(ProjectError::Runtime {
                module,
                path,
                error: Box::new(error),
            })
        }
    }
}

fn module_for_error(interpreter: &Interpreter, fallback: &ModuleId) -> ModuleId {
    if let Ok(module) = ModuleId::parse(interpreter.active_module()) {
        return module;
    }
    fallback.clone()
}

fn project_root(entry: &Path) -> PathBuf {
    let canonical = fs::canonicalize(entry).unwrap_or_else(|_| entry.to_path_buf());
    if let Ok(current) = env::current_dir() {
        let current = fs::canonicalize(&current).unwrap_or(current);
        if canonical.starts_with(&current) {
            return current;
        }
    }
    entry
        .parent()
        .unwrap_or_else(|| Path::new("."))
        .to_path_buf()
}

#[derive(Debug)]
struct SourceSet {
    root: PathBuf,
    from_disk: bool,
    files: BTreeMap<ModuleId, (PathBuf, String)>,
    case_ids: BTreeMap<String, ModuleId>,
}

impl SourceSet {
    fn from_disk(root: PathBuf) -> Self {
        Self {
            root,
            from_disk: true,
            files: BTreeMap::new(),
            case_ids: BTreeMap::new(),
        }
    }

    fn from_memory(root: PathBuf) -> Self {
        Self {
            root,
            from_disk: false,
            files: BTreeMap::new(),
            case_ids: BTreeMap::new(),
        }
    }

    fn ensure(
        &mut self,
        id: &ModuleId,
        import: Option<(ModuleId, PathBuf, Span)>,
    ) -> Result<(), ProjectError> {
        if self.files.contains_key(id) {
            return Ok(());
        }
        if !self.from_disk {
            return Err(missing_module(import, id));
        }

        let path = self.root.join(id.as_str());
        let bytes = fs::read(&path).map_err(|error| {
            if error.kind() == std::io::ErrorKind::NotFound {
                match import.clone() {
                    Some(_) => missing_module(import.clone(), id),
                    None => io_error(
                        None,
                        &path,
                        "NS9008",
                        format!("Unable to read the entry module '{}': {error}", id),
                    ),
                }
            } else {
                io_error(
                    import.clone().map(|(module, path, _)| (module, path)),
                    &path,
                    "NS9008",
                    format!("Unable to read module '{}': {error}", id),
                )
            }
        })?;
        let canonical = fs::canonicalize(&path).map_err(|error| {
            io_error(
                import.clone().map(|(module, path, _)| (module, path)),
                &path,
                "NS9008",
                format!("Unable to canonicalize module '{}': {error}", id),
            )
        })?;
        if !canonical.starts_with(&self.root) {
            return Err(io_error(
                import.map(|(module, path, _)| (module, path)),
                &path,
                "NS3012",
                format!("Module '{}' escapes the project root.", id),
            ));
        }

        let relative = canonical
            .strip_prefix(&self.root)
            .expect("containment was checked");
        let actual = module_id_from_path(relative).map_err(|message| {
            io_error(
                None,
                &path,
                "NS3012",
                format!("Module '{}' has an invalid canonical id: {message}.", id),
            )
        })?;
        if actual != *id {
            return Err(io_error(
                None,
                &path,
                "NS3012",
                format!(
                    "Module import '{}' resolves to '{}'. Import paths must match the canonical file path, including case.",
                    id.as_str(),
                    actual.as_str()
                ),
            ));
        }
        if let Some(existing) = self.case_ids.get(&id.as_str().to_lowercase())
            && existing != id
        {
            return Err(io_error(
                None,
                &path,
                "NS3012",
                format!(
                    "Module ids '{}' and '{}' collide case-insensitively.",
                    existing.as_str(),
                    id.as_str()
                ),
            ));
        }
        self.case_ids.insert(id.as_str().to_lowercase(), id.clone());

        let text = String::from_utf8(bytes).map_err(|_| {
            io_error(
                import.map(|(module, path, _)| (module, path)),
                &path,
                "NS9007",
                format!("Module '{}' is not valid UTF-8.", id),
            )
        })?;
        self.files.insert(id.clone(), (path, text));
        Ok(())
    }

    fn text(&self, id: &ModuleId) -> &str {
        &self
            .files
            .get(id)
            .expect("source must be ensured before discovery")
            .1
    }

    fn path(&self, id: &ModuleId) -> PathBuf {
        self.files
            .get(id)
            .map_or_else(|| PathBuf::from(id.as_str()), |(path, _)| path.clone())
    }
}

fn assemble(
    entry: ModuleId,
    mode: Mode,
    infer_mode: bool,
    mut sources: SourceSet,
) -> Result<Project, ProjectError> {
    let mut parsed: BTreeMap<ModuleId, ast::Module> = BTreeMap::new();
    let mut graph = ModuleGraph::default();
    let mut visiting = Vec::new();
    let mut done = BTreeSet::new();
    let mut order = Vec::new();
    let mut diagnostics = Vec::new();

    discover(
        &entry,
        &mut sources,
        &mut parsed,
        &mut graph,
        &mut visiting,
        &mut done,
        &mut order,
        &mut diagnostics,
    )?;

    if !diagnostics.is_empty() {
        return Err(diagnostic_error(diagnostics));
    }

    let mode = if infer_mode {
        let mut inferences = Vec::new();
        for (id, module) in &parsed {
            let inference = infer_module_mode_named(module, id.as_str());
            if !inference.is_ok() {
                diagnostics.extend(inference.diagnostics.iter().cloned().map(|diagnostic| {
                    ProjectDiagnostic {
                        module: id.clone(),
                        path: sources.path(id),
                        diagnostic,
                    }
                }));
            }
            inferences.push(inference);
        }
        let joined = infer_project(&inferences);
        let resolved = joined.resolved_mode();
        if !joined.is_ok() {
            diagnostics.extend(joined.diagnostics.into_iter().map(|diagnostic| {
                ProjectDiagnostic {
                    module: entry.clone(),
                    path: sources.path(&entry),
                    diagnostic,
                }
            }));
        }
        if !diagnostics.is_empty() {
            return Err(diagnostic_error(diagnostics));
        }
        resolved
    } else {
        mode
    };

    let resolver = ModuleResolver::from_graph(&graph);
    let mut surfaces: BTreeMap<String, ModuleTypeSurface> = BTreeMap::new();
    let mut checked: BTreeMap<ModuleId, CheckResult> = BTreeMap::new();

    for id in &order {
        let module = parsed.get(id).expect("order only names parsed modules");
        let resolved = match resolver.resolve_root(id.as_str()) {
            Ok(resolved) => resolved,
            Err(errors) => {
                diagnostics.extend(errors.into_iter().map(|diagnostic| ProjectDiagnostic {
                    module: id.clone(),
                    path: sources.path(id),
                    diagnostic,
                }));
                continue;
            }
        };
        let options = CheckOptions::with_surfaces(mode, surfaces.clone());
        match check_resolved(module, &resolved, options) {
            Ok(result) => {
                let surface = module_type_surface_with_surfaces(&result, &surfaces);
                surfaces.insert(id.to_string(), surface);
                checked.insert(id.clone(), result);
            }
            Err(errors) => {
                diagnostics.extend(errors.into_iter().map(|diagnostic| ProjectDiagnostic {
                    module: id.clone(),
                    path: sources.path(id),
                    diagnostic,
                }));
            }
        }
    }

    if !diagnostics.is_empty() {
        return Err(diagnostic_error(diagnostics));
    }

    let mut modules = BTreeMap::new();
    for (id, module) in parsed {
        let resolved = resolver.resolve_root(id.as_str()).map_err(|errors| {
            ProjectError::Diagnostics(
                errors
                    .into_iter()
                    .map(|diagnostic| ProjectDiagnostic {
                        module: id.clone(),
                        path: sources.path(&id),
                        diagnostic,
                    })
                    .collect(),
            )
        })?;
        let checked = checked.remove(&id).ok_or_else(|| ProjectError::Runtime {
            module: id.clone(),
            path: sources.path(&id),
            error: Box::new(NovaError::Internal(format!(
                "module '{id}' was not checked before project assembly"
            ))),
        })?;
        let (path, text) = sources
            .files
            .remove(&id)
            .expect("parsed module has source text");
        modules.insert(
            id.clone(),
            CompiledModule {
                id,
                path,
                text: Rc::from(text),
                ast: module,
                resolved,
                checked,
            },
        );
    }

    Ok(Project {
        root: sources.root,
        entry,
        mode,
        modules,
    })
}

#[allow(clippy::too_many_arguments)]
fn discover(
    id: &ModuleId,
    sources: &mut SourceSet,
    parsed: &mut BTreeMap<ModuleId, ast::Module>,
    graph: &mut ModuleGraph,
    visiting: &mut Vec<ModuleId>,
    done: &mut BTreeSet<ModuleId>,
    order: &mut Vec<ModuleId>,
    diagnostics: &mut Vec<ProjectDiagnostic>,
) -> Result<(), ProjectError> {
    if done.contains(id) {
        return Ok(());
    }
    if let Some(start) = visiting.iter().position(|active| active == id) {
        let cycle = visiting[start..]
            .iter()
            .chain(std::iter::once(id))
            .map(ModuleId::as_str)
            .collect::<Vec<_>>()
            .join(" -> ");
        let path = sources.path(id);
        diagnostics.push(ProjectDiagnostic {
            module: id.clone(),
            path,
            diagnostic: CoreDiagnostic::error("NS3013", format!("Cyclic module import: {cycle}."))
                .with_hint("Break the direct or transitive import cycle."),
        });
        return Ok(());
    }

    sources.ensure(id, None)?;
    let text = sources.text(id).to_owned();
    let module = match parse(&text) {
        Ok(module) => module,
        Err(errors) => {
            diagnostics.extend(errors.into_iter().map(|diagnostic| ProjectDiagnostic {
                module: id.clone(),
                path: sources.path(id),
                diagnostic,
            }));
            done.insert(id.clone());
            return Ok(());
        }
    };
    graph.insert_module(id.as_str(), module.clone());
    parsed.insert(id.clone(), module.clone());
    visiting.push(id.clone());

    let mut local_targets = BTreeMap::new();
    for specifier in collect_module_specifiers(&module) {
        match resolve_specifier(id, &specifier.specifier) {
            Ok(Some(target)) => {
                local_targets.insert(target, (specifier.specifier, specifier.span));
            }
            Ok(None) => {
                return Err(ProjectError::Runtime {
                    module: id.clone(),
                    path: sources.path(id),
                    error: Box::new(NovaError::Unsupported {
                        message: format!(
                            "External module '{}' has no Nova adapter yet. Local project modules and relative '.ns' paths are supported.",
                            specifier.specifier
                        ),
                        span: Some(specifier.span),
                    }),
                });
            }
            Err(diagnostic) => {
                diagnostics.push(ProjectDiagnostic {
                    module: id.clone(),
                    path: sources.path(id),
                    diagnostic,
                });
            }
        }
    }

    for (target, (specifier, span)) in local_targets {
        if visiting.contains(&target) {
            let path = sources.path(id);
            diagnostics.push(ProjectDiagnostic {
                module: id.clone(),
                path,
                diagnostic: CoreDiagnostic::error(
                    "NS3013",
                    format!(
                        "Cyclic module import: '{}' imports '{}' while it is still loading.",
                        id.as_str(),
                        specifier
                    ),
                )
                .with_span(span)
                .with_hint("Break the direct or transitive import cycle."),
            });
            continue;
        }
        graph.insert_import(id.as_str(), specifier, target.as_str());
        sources.ensure(&target, Some((id.clone(), sources.path(id), span)))?;
        discover(
            &target,
            sources,
            parsed,
            graph,
            visiting,
            done,
            order,
            diagnostics,
        )?;
    }

    visiting.pop();
    done.insert(id.clone());
    order.push(id.clone());
    Ok(())
}

fn resolve_specifier(importer: &ModuleId, raw: &str) -> Result<Option<ModuleId>, Diagnostic> {
    if raw.starts_with("std:") {
        return Ok(None);
    }
    if raw.starts_with('/') || raw.contains(':') || raw.contains('\\') {
        return Err(Diagnostic::error(
            "NS3012",
            format!("Absolute module specifier '{raw}' is not allowed."),
        )
        .with_hint("Use a relative '.ns' path or a dotted project path."));
    }

    if raw.starts_with("./") || raw.starts_with("../") {
        let base = importer.parent();
        let joined = if base.is_empty() {
            raw.to_owned()
        } else {
            format!("{base}/{raw}")
        };
        return normalize_path(&joined).map(Some).map_err(|message| {
            Diagnostic::error(
                "NS3012",
                format!("Invalid module specifier '{raw}': {message}."),
            )
            .with_hint("Local modules must stay inside the project root.")
        });
    }

    if raw.contains('/') || raw.starts_with('@') || !raw.contains('.') {
        return Ok(None);
    }

    if raw.ends_with(".ns") {
        return Ok(None);
    }

    if raw
        .split('.')
        .all(|segment| !segment.is_empty() && is_identifier(segment))
    {
        let path = format!("{}.ns", raw.replace('.', "/"));
        return normalize_path(&path).map(Some).map_err(|message| {
            Diagnostic::error(
                "NS3012",
                format!("Invalid module specifier '{raw}': {message}."),
            )
        });
    }

    Ok(None)
}

fn normalize_path(path: &str) -> Result<ModuleId, String> {
    let mut components: Vec<&str> = Vec::new();
    for component in path.split('/') {
        match component {
            "" | "." => {}
            ".." => {
                if components.pop().is_none() {
                    return Err("the path escapes the project root".to_owned());
                }
            }
            value => components.push(value),
        }
    }
    let joined = components.join("/");
    if joined.is_empty() {
        return Err("the path does not name a module".to_owned());
    }
    if !joined.ends_with(".ns") {
        return Err("local module paths must end in '.ns'".to_owned());
    }
    ModuleId::parse(joined.clone()).map_err(|error| error.to_string())
}

fn is_identifier(value: &str) -> bool {
    let mut characters = value.chars();
    let Some(first) = characters.next() else {
        return false;
    };
    (first == '_' || first.is_alphabetic())
        && characters.all(|character| character == '_' || character.is_alphanumeric())
}

fn module_id_from_path(path: &Path) -> Result<ModuleId, String> {
    let mut parts = Vec::new();
    for component in path.components() {
        let std::path::Component::Normal(part) = component else {
            return Err("module path contains a non-normal component".to_owned());
        };
        let Some(part) = part.to_str() else {
            return Err("module path is not valid UTF-8".to_owned());
        };
        parts.push(part.to_owned());
    }
    let joined = parts.join("/");
    ModuleId::parse(joined).map_err(|error| error.to_string())
}

fn missing_module(import: Option<(ModuleId, PathBuf, Span)>, id: &ModuleId) -> ProjectError {
    let (module, path, span) =
        import.unwrap_or_else(|| (id.clone(), PathBuf::from(id.as_str()), Span::default()));
    ProjectError::Diagnostics(vec![ProjectDiagnostic {
        module,
        path,
        diagnostic: Diagnostic::error(
            "NS3012",
            format!("Imported module '{}' could not be resolved.", id.as_str()),
        )
        .with_span(span)
        .with_hint("Create the module file or fix the import specifier."),
    }])
}

fn diagnostic_error(mut diagnostics: Vec<ProjectDiagnostic>) -> ProjectError {
    diagnostics.sort_by(|left, right| {
        (
            left.module.as_str(),
            left.diagnostic.span.map_or(0, |span| span.start),
            left.diagnostic.code.as_str(),
            left.diagnostic.message.as_str(),
        )
            .cmp(&(
                right.module.as_str(),
                right.diagnostic.span.map_or(0, |span| span.start),
                right.diagnostic.code.as_str(),
                right.diagnostic.message.as_str(),
            ))
    });
    diagnostics.dedup();
    ProjectError::Diagnostics(diagnostics)
}

fn io_error(
    module: Option<(ModuleId, PathBuf)>,
    path: &Path,
    code: &'static str,
    message: String,
) -> ProjectError {
    let (module, diagnostic_path) =
        module.unwrap_or_else(|| (ModuleId("<entry>".to_owned()), path.to_path_buf()));
    ProjectError::Diagnostics(vec![ProjectDiagnostic {
        module,
        path: diagnostic_path,
        diagnostic: Diagnostic::error(code, message),
    }])
}
