use std::collections::BTreeMap;

use astra_capabilities::{Mode, ModeInference, infer_module_mode_named, infer_project};
#[allow(unused_imports)]
use astra_codegen_c::{CModule, IntegerPolicy as CIntegerPolicy};
use astra_core::checker::{CheckOptions, TypeAnnotation, check_resolved};
use astra_core::diagnostic::Diagnostic;
use astra_core::formatter::format_source;
use astra_core::ir::Program;
use astra_core::parser::parse;
use astra_core::resolver::resolve_module;
use astra_core::resolver::{ModuleGraph, collect_module_specifiers};
use astra_core::type_surface::{ProjectModule, check_module_graph_with_imports};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

pub const API_VERSION: &str = "1.0";

pub const API_VERSION_MAJOR: u32 = 1;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SourceUnit {
    pub path: String,
    pub text: String,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum IntegerPolicy {
    #[default]
    Wrapping,

    SafeChecked,

    Arbitrary,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ModuleIr {
    pub key: String,

    pub path: String,

    pub ast: astra_core::ast::Module,

    pub hir: astra_core::hir::Module,

    pub annotations: Vec<TypeAnnotation>,

    pub manual_memory_scopes: Vec<astra_core::span::Span>,

    pub integer_policy: IntegerPolicy,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CompileRequest {
    pub api_version: String,
    pub source: SourceUnit,
    pub target: String,

    #[serde(default)]
    pub mode: Option<Mode>,

    #[serde(default)]
    pub integer_policy: IntegerPolicy,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CompileResponse {
    pub api_version: String,
    pub program: Option<Program>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub ir: Option<ModuleIr>,
    pub diagnostics: Vec<Diagnostic>,
    pub metadata: Value,
    pub artifacts: Vec<Value>,
}

impl CompileResponse {
    #[must_use]
    pub fn success(program: Program) -> Self {
        Self {
            api_version: API_VERSION.to_owned(),
            program: Some(program),
            ir: None,
            diagnostics: Vec::new(),
            metadata: json!({}),
            artifacts: Vec::new(),
        }
    }

    #[must_use]
    pub fn with_ir(mut self, ir: ModuleIr) -> Self {
        self.ir = Some(ir);
        self
    }

    #[must_use]
    pub fn with_metadata(mut self, metadata: Value) -> Self {
        self.metadata = metadata;
        self
    }

    #[must_use]
    pub fn with_artifact(mut self, artifact: Value) -> Self {
        self.artifacts.push(artifact);
        self
    }

    #[must_use]
    pub fn failure(diagnostics: Vec<Diagnostic>) -> Self {
        Self {
            api_version: API_VERSION.to_owned(),
            program: None,
            ir: None,
            diagnostics,
            metadata: json!({}),
            artifacts: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FormatResponse {
    pub api_version: String,
    pub source: Option<String>,
    pub diagnostics: Vec<Diagnostic>,
}

impl FormatResponse {
    #[must_use]
    pub fn success(source: String) -> Self {
        Self {
            api_version: API_VERSION.to_owned(),
            source: Some(source),
            diagnostics: Vec::new(),
        }
    }

    #[must_use]
    pub fn failure(diagnostics: Vec<Diagnostic>) -> Self {
        Self {
            api_version: API_VERSION.to_owned(),
            source: None,
            diagnostics,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AstResponse {
    pub api_version: String,
    pub ok: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ast: Option<astra_core::ast::Module>,
    pub diagnostics: Vec<Diagnostic>,
}

impl AstResponse {
    #[must_use]
    pub fn success(ast: astra_core::ast::Module) -> Self {
        Self {
            api_version: API_VERSION.to_owned(),
            ok: true,
            ast: Some(ast),
            diagnostics: Vec::new(),
        }
    }

    #[must_use]
    pub fn failure(diagnostics: Vec<Diagnostic>) -> Self {
        Self {
            api_version: API_VERSION.to_owned(),
            ok: false,
            ast: None,
            diagnostics,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExplainResponse {
    pub api_version: String,
    pub ok: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub inference: Option<ModeInference>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub explanation: Option<String>,
    pub diagnostics: Vec<Diagnostic>,
}

impl ExplainResponse {
    #[must_use]
    pub fn success(inference: ModeInference) -> Self {
        let explanation = inference.explain();
        Self {
            api_version: API_VERSION.to_owned(),
            ok: true,
            inference: Some(inference),
            explanation: Some(explanation),
            diagnostics: Vec::new(),
        }
    }

    #[must_use]
    pub fn failure(diagnostics: Vec<Diagnostic>) -> Self {
        Self {
            api_version: API_VERSION.to_owned(),
            ok: false,
            inference: None,
            explanation: None,
            diagnostics,
        }
    }
}

fn module_stem(path: &str) -> &str {
    let base = path.rsplit(&['/', '\\'][..]).next().unwrap_or(path);
    base.strip_suffix(".ns").unwrap_or(base)
}

pub trait CompilerApi {
    fn check(&self, request: &CompileRequest) -> CompileResponse;
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProjectCheckRequest {
    pub api_version: String,

    pub sources: Vec<ProjectSourceUnit>,
    pub target: String,

    #[serde(default)]
    pub mode: Option<Mode>,

    #[serde(default)]
    pub integer_policy: IntegerPolicy,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProjectSourceUnit {
    pub key: String,

    pub path: String,

    pub text: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ModuleCheckResult {
    pub key: String,
    pub path: String,
    pub diagnostics: Vec<Diagnostic>,
    pub ok: bool,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub ir: Option<ModuleIr>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProjectCheckResponse {
    pub api_version: String,
    pub ok: bool,
    pub modules: Vec<ModuleCheckResult>,
    pub program: Option<Program>,
    pub metadata: Value,
}

pub fn check_project(request: &ProjectCheckRequest) -> ProjectCheckResponse {
    if request.api_version != API_VERSION {
        return ProjectCheckResponse {
            api_version: API_VERSION.to_owned(),
            ok: false,
            modules: Vec::new(),
            program: None,
            metadata: json!({ "error": "unsupported apiVersion" }),
        };
    }

    let mut project_modules: Vec<ProjectModule> = Vec::new();
    let mut ast_map: BTreeMap<String, astra_core::ast::Module> = BTreeMap::new();
    let mut path_map: BTreeMap<String, String> = BTreeMap::new();
    let mut early_failures: Vec<ModuleCheckResult> = Vec::new();

    for unit in &request.sources {
        path_map.insert(unit.key.clone(), unit.path.clone());
        match parse(&unit.text) {
            Ok(module) => {
                ast_map.insert(unit.key.clone(), module.clone());
                project_modules.push(ProjectModule {
                    key: unit.key.clone(),
                    source: module,
                });
            }
            Err(diagnostics) => {
                early_failures.push(ModuleCheckResult {
                    key: unit.key.clone(),
                    path: unit.path.clone(),
                    diagnostics,
                    ok: false,
                    ir: None,
                });
            }
        }
    }

    if !early_failures.is_empty() {
        return ProjectCheckResponse {
            api_version: API_VERSION.to_owned(),
            ok: false,
            modules: early_failures,
            program: None,
            metadata: json!({
                "moduleOrder": request.sources.iter().map(|s| &s.key).collect::<Vec<_>>(),
                "target": request.target,
                "mode": request.mode.unwrap_or(Mode::Core),
                "modeSource": if request.mode.is_some() { "explicit" } else { "inferred" },
            }),
        };
    }

    let inferences: Vec<ModeInference> = project_modules
        .iter()
        .map(|module| infer_module_mode_named(&module.source, &module.key))
        .collect();
    let project_inference = infer_project(&inferences);
    let mode_source = if request.mode.is_some() {
        "explicit"
    } else {
        "inferred"
    };
    let selected_mode = request
        .mode
        .unwrap_or_else(|| project_inference.resolved_mode());
    let inferred_modes: BTreeMap<String, Mode> = inferences
        .iter()
        .map(|inference| {
            (
                inference
                    .module
                    .clone()
                    .unwrap_or_else(|| "<module>".to_owned()),
                inference.resolved_mode(),
            )
        })
        .collect();
    let metadata = json!({
        "moduleOrder": request.sources.iter().map(|s| &s.key).collect::<Vec<_>>(),
        "target": request.target,
        "mode": selected_mode,
        "modeSource": mode_source,
        "requiredMode": project_inference.required_mode,
        "inferredModes": inferred_modes,
        "modeReasons": project_inference.reasons,
        "modeExplanation": project_inference.explain(),
    });

    if request.mode.is_none() && !project_inference.is_ok() {
        let mut module_results: Vec<ModuleCheckResult> = request
            .sources
            .iter()
            .map(|unit| {
                let inference = inferences
                    .iter()
                    .find(|inference| inference.module.as_deref() == Some(unit.key.as_str()));
                ModuleCheckResult {
                    key: unit.key.clone(),
                    path: unit.path.clone(),
                    diagnostics: inference
                        .map(|inference| inference.diagnostics.clone())
                        .unwrap_or_default(),
                    ok: inference.is_none_or(ModeInference::is_ok),
                    ir: None,
                }
            })
            .collect();
        let mut project_diagnostics = project_inference.diagnostics.clone();
        if let Some(last) = module_results.last_mut() {
            for diagnostic in project_diagnostics.drain(..) {
                if !last.diagnostics.contains(&diagnostic) {
                    last.diagnostics.push(diagnostic);
                }
            }
            last.ok = false;
        }
        return ProjectCheckResponse {
            api_version: API_VERSION.to_owned(),
            ok: false,
            modules: module_results,
            program: None,
            metadata,
        };
    }

    let mut module_graph = ModuleGraph::default();
    for module in &project_modules {
        module_graph.insert_module(module.key.clone(), module.source.clone());
    }
    for module in &project_modules {
        for specifier in collect_module_specifiers(&module.source) {
            let target = resolve_project_target(
                &module.key,
                &specifier.specifier,
                &module_graph,
                &BTreeMap::new(),
            );
            module_graph.insert_import(module.key.clone(), specifier.specifier, target);
        }
    }

    let ordered_keys = dependency_order(&project_modules, &module_graph);
    let ordered_modules: Vec<ProjectModule> = ordered_keys
        .iter()
        .filter_map(|key| {
            project_modules
                .iter()
                .find(|module| module.key == *key)
                .cloned()
        })
        .collect();
    let graph = check_module_graph_with_imports(
        &ordered_modules,
        &module_graph,
        CheckOptions::new(selected_mode),
    );

    let ok = graph.is_ok();

    let mut module_results: BTreeMap<String, ModuleCheckResult> = BTreeMap::new();
    for (key, result) in &graph.modules {
        let ir = ast_map.get(key).map(|ast| ModuleIr {
            key: key.clone(),
            path: path_map.get(key).cloned().unwrap_or_else(|| key.clone()),
            ast: ast.clone(),
            hir: result.module.clone(),
            annotations: result.annotations.clone(),
            manual_memory_scopes: result.manual_memory_scopes.clone(),
            integer_policy: request.integer_policy,
        });
        module_results.insert(
            key.clone(),
            ModuleCheckResult {
                key: key.clone(),
                path: path_map.get(key).cloned().unwrap_or_else(|| key.clone()),
                diagnostics: Vec::new(),
                ok: true,
                ir,
            },
        );
    }
    for (key, diagnostics) in &graph.diagnostics {
        module_results.insert(
            key.clone(),
            ModuleCheckResult {
                key: key.clone(),
                path: path_map.get(key).cloned().unwrap_or_else(|| key.clone()),
                diagnostics: diagnostics.clone(),
                ok: false,
                ir: None,
            },
        );
    }

    let modules: Vec<ModuleCheckResult> = request
        .sources
        .iter()
        .filter_map(|source| module_results.remove(&source.key))
        .collect();

    let program = ok.then(|| {
        let last_key = project_modules
            .last()
            .map(|m| m.key.clone())
            .unwrap_or_default();
        Program { name: last_key }
    });

    let module_order: Vec<&str> = request.sources.iter().map(|s| s.key.as_str()).collect();

    ProjectCheckResponse {
        api_version: API_VERSION.to_owned(),
        ok,
        modules,
        program,
        metadata: json!({
            "moduleOrder": module_order,
            "target": request.target,
            "mode": selected_mode,
            "modeSource": mode_source,
            "requiredMode": project_inference.required_mode,
            "inferredModes": inferred_modes,
            "modeReasons": project_inference.reasons,
            "modeExplanation": project_inference.explain(),
        }),
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProjectCompileRequest {
    pub api_version: String,

    pub sources: Vec<ProjectSourceUnit>,
    pub target: String,

    #[serde(default)]
    pub mode: Option<Mode>,

    #[serde(default)]
    pub integer_policy: IntegerPolicy,

    #[serde(default)]
    pub library: bool,

    #[serde(default)]
    pub import_map: BTreeMap<String, String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ModuleCompileArtifact {
    pub key: String,
    pub path: String,

    pub c_filename: String,

    pub h_filename: String,
    pub c_source: String,
    pub h_source: String,

    pub script_sources: Vec<(String, String)>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProjectCompileResponse {
    pub api_version: String,
    pub ok: bool,
    pub modules: Vec<ModuleCompileArtifact>,

    pub host_shim: String,
    pub diagnostics: Vec<Diagnostic>,
    pub metadata: Value,
}

pub fn compile_project(request: &ProjectCompileRequest) -> ProjectCompileResponse {
    if request.api_version != API_VERSION {
        return ProjectCompileResponse {
            api_version: API_VERSION.to_owned(),
            ok: false,
            modules: Vec::new(),
            host_shim: String::new(),
            diagnostics: vec![Diagnostic::error(
                "NS9009",
                format!(
                    "Unsupported apiVersion '{}'; expected '{}'.",
                    request.api_version, API_VERSION
                ),
            )],
            metadata: json!({ "error": "unsupported apiVersion" }),
        };
    }

    if request.sources.is_empty() {
        return ProjectCompileResponse {
            api_version: API_VERSION.to_owned(),
            ok: false,
            modules: Vec::new(),
            host_shim: String::new(),
            diagnostics: vec![Diagnostic::error(
                "NS9010",
                "compile_project requires at least one source unit",
            )],
            metadata: json!({}),
        };
    }

    let mut project_modules: Vec<astra_core::type_surface::ProjectModule> = Vec::new();
    let mut path_map: std::collections::BTreeMap<String, String> =
        std::collections::BTreeMap::new();
    let mut parse_errors: Vec<Diagnostic> = Vec::new();

    for unit in &request.sources {
        path_map.insert(unit.key.clone(), unit.path.clone());
        match astra_core::parser::parse(&unit.text) {
            Ok(module) => {
                project_modules.push(astra_core::type_surface::ProjectModule {
                    key: unit.key.clone(),
                    source: module,
                });
            }
            Err(diagnostics) => {
                parse_errors.extend(diagnostics);
            }
        }
    }

    if !parse_errors.is_empty() {
        return ProjectCompileResponse {
            api_version: API_VERSION.to_owned(),
            ok: false,
            modules: Vec::new(),
            host_shim: String::new(),
            diagnostics: parse_errors,
            metadata: json!({}),
        };
    }

    use astra_capabilities::{ModeInference, infer_module_mode_named, infer_project};
    let inferences: Vec<ModeInference> = project_modules
        .iter()
        .map(|m| infer_module_mode_named(&m.source, &m.key))
        .collect();
    let project_inference = infer_project(&inferences);
    let selected_mode = request
        .mode
        .unwrap_or_else(|| project_inference.resolved_mode());

    if request.mode.is_none() && !project_inference.is_ok() {
        return ProjectCompileResponse {
            api_version: API_VERSION.to_owned(),
            ok: false,
            modules: Vec::new(),
            host_shim: String::new(),
            diagnostics: project_inference.diagnostics.clone(),
            metadata: json!({}),
        };
    }

    let mut module_graph = ModuleGraph::default();
    for module in &project_modules {
        module_graph.insert_module(module.key.clone(), module.source.clone());
    }
    for module in &project_modules {
        for specifier in collect_module_specifiers(&module.source) {
            let target = resolve_project_target(
                &module.key,
                &specifier.specifier,
                &module_graph,
                &request.import_map,
            );
            module_graph.insert_import(module.key.clone(), specifier.specifier, target);
        }
    }
    let ordered_keys = dependency_order(&project_modules, &module_graph);
    let ordered_modules: Vec<astra_core::type_surface::ProjectModule> = ordered_keys
        .iter()
        .filter_map(|key| {
            project_modules
                .iter()
                .find(|module| module.key == *key)
                .cloned()
        })
        .collect();
    let graph = astra_core::type_surface::check_module_graph_with_imports(
        &ordered_modules,
        &module_graph,
        astra_core::checker::CheckOptions::new(selected_mode),
    );

    if !graph.is_ok() {
        return ProjectCompileResponse {
            api_version: API_VERSION.to_owned(),
            ok: false,
            modules: Vec::new(),
            host_shim: String::new(),
            diagnostics: graph.all_diagnostics(),
            metadata: json!({}),
        };
    }

    let c_ip = match request.integer_policy {
        IntegerPolicy::Wrapping => CIntegerPolicy::Wrapping,
        IntegerPolicy::SafeChecked => CIntegerPolicy::SafeChecked,
        IntegerPolicy::Arbitrary => CIntegerPolicy::Arbitrary,
    };

    let entries: Vec<astra_codegen_c::ProjectEntry> = project_modules
        .iter()
        .filter_map(|pm| {
            let checked = graph.get(&pm.key)?;
            let dep_keys: Vec<String> = checked
                .module
                .imports
                .iter()
                .map(|imp| imp.module.clone())
                .collect();
            Some(astra_codegen_c::ProjectEntry {
                module: CModule {
                    name: checked.module.name.clone(),
                    ast: pm.source.clone(),
                    hir: checked.module.clone(),
                    annotations: checked.annotations.clone(),
                    manual_memory_scopes: checked.manual_memory_scopes.clone(),
                    integer_policy: c_ip,
                },
                key: pm.key.clone(),
                dep_keys,
            })
        })
        .collect();

    match astra_codegen_c::generate_project_with_kind(
        &entries,
        if request.library {
            astra_codegen_c::EmitKind::Library
        } else {
            astra_codegen_c::EmitKind::Executable
        },
    ) {
        Ok(generated) => {
            let mut modules = Vec::new();
            let mut host_shim = String::new();
            for (entry, module_out) in entries.iter().zip(generated.iter()) {
                let stem = entry
                    .key
                    .strip_suffix(".ns")
                    .unwrap_or(&entry.key)
                    .replace(['/', '.', '-'], "_");
                let c_filename = format!("{stem}.c");
                let h_filename = format!("{stem}.h");
                modules.push(ModuleCompileArtifact {
                    key: entry.key.clone(),
                    path: path_map
                        .get(&entry.key)
                        .cloned()
                        .unwrap_or_else(|| entry.key.clone()),
                    c_filename,
                    h_filename,
                    c_source: module_out.source.clone(),
                    h_source: module_out.header.clone().unwrap_or_default(),
                    script_sources: module_out.script_sources.clone(),
                });
                if host_shim.is_empty() {
                    host_shim = module_out.host_shim.clone();
                }
            }
            ProjectCompileResponse {
                api_version: API_VERSION.to_owned(),
                ok: true,
                modules,
                host_shim,
                diagnostics: Vec::new(),
                metadata: json!({
                    "moduleCount": entries.len(),
                    "mode": selected_mode,
                    "target": request.target,
                    "integerPolicy": request.integer_policy,
                }),
            }
        }
        Err(backend_error) => ProjectCompileResponse {
            api_version: API_VERSION.to_owned(),
            ok: false,
            modules: Vec::new(),
            host_shim: String::new(),
            diagnostics: vec![Diagnostic::error(
                backend_error.code.clone(),
                backend_error.message.clone(),
            )],
            metadata: json!({}),
        },
    }
}

#[derive(Debug, Default, Clone, Copy)]
pub struct FrontendCompiler;

impl FrontendCompiler {
    #[must_use]
    pub fn compile(&self, request: &CompileRequest) -> CompileResponse {
        if !matches!(request.target.as_str(), "js" | "ts" | "c") {
            return CompileResponse::failure(vec![
                Diagnostic::error(
                    "NS8001",
                    format!(
                        "Target '{}' is not supported by the typed source backend.",
                        request.target
                    ),
                )
                .with_hint("Use target 'js', 'ts', or 'c'."),
            ])
            .with_metadata(json!({ "target": request.target }));
        }

        self.run(request, true)
    }

    fn run(&self, request: &CompileRequest, typed: bool) -> CompileResponse {
        match parse(&request.source.text) {
            Ok(module) => {
                let inference = infer_module_mode_named(&module, &request.source.path);
                let mode_source = if request.mode.is_some() {
                    "explicit"
                } else {
                    "inferred"
                };
                let selected_mode = request.mode.unwrap_or_else(|| inference.resolved_mode());
                if request.mode.is_none() && !inference.is_ok() {
                    return CompileResponse::failure(inference.diagnostics.clone()).with_metadata(
                        json!({
                            "mode": selected_mode,
                            "modeSource": mode_source,
                            "requiredMode": inference.required_mode,
                            "modeReasons": inference.reasons,
                            "modeExplanation": inference.explain(),
                        }),
                    );
                }

                match resolve_module(&module) {
                    Ok(resolved) => {
                        match check_resolved(&module, &resolved, CheckOptions::new(selected_mode)) {
                            Ok(checked) => {
                                let name = checked.module.name.clone();
                                let metadata = json!({
                                    "mode": selected_mode,
                                    "modeSource": mode_source,
                                    "requiredMode": inference.required_mode,
                                    "modeReasons": inference.reasons,
                                    "modeExplanation": inference.explain(),
                                    "target": request.target,
                                    "integerPolicy": request.integer_policy,
                                });
                                let mut response = CompileResponse::success(Program { name })
                                    .with_metadata(metadata);
                                if typed {
                                    let ir = ModuleIr {
                                        key: request.source.path.clone(),
                                        path: request.source.path.clone(),
                                        ast: module.clone(),
                                        hir: checked.module.clone(),
                                        annotations: checked.annotations.clone(),
                                        manual_memory_scopes: checked.manual_memory_scopes.clone(),
                                        integer_policy: request.integer_policy,
                                    };

                                    if request.target == "c" {
                                        let c_module = CModule {
                                            name: checked.module.name.clone(),
                                            ast: module,
                                            hir: checked.module,
                                            annotations: checked.annotations,
                                            manual_memory_scopes: checked.manual_memory_scopes,
                                            integer_policy: match request.integer_policy {
                                                IntegerPolicy::Wrapping => CIntegerPolicy::Wrapping,
                                                IntegerPolicy::SafeChecked => {
                                                    CIntegerPolicy::SafeChecked
                                                }
                                                IntegerPolicy::Arbitrary => {
                                                    CIntegerPolicy::Arbitrary
                                                }
                                            },
                                        };
                                        match astra_codegen_c::generate(&c_module) {
                                            Ok(generated) => {
                                                response = response
                                                    .with_ir(ir)
                                                    .with_artifact(json!({
                                                        "type": "c-source",
                                                        "name": format!("{}.c", module_stem(&request.source.path)),
                                                        "filename": format!("{}.c", module_stem(&request.source.path)),
                                                        "content": generated.source,
                                                    }))
                                                    .with_artifact(json!({
                                                        "type": "c-host-shim",
                                                        "name": "astra_host.c",
                                                        "filename": "astra_host.c",
                                                        "content": generated.host_shim,
                                                    }));
                                                for (filename, content) in generated.script_sources
                                                {
                                                    response = response.with_artifact(json!({
                                                        "type": "c-script",
                                                        "name": filename,
                                                        "filename": filename,
                                                        "content": content,
                                                    }));
                                                }
                                            }
                                            Err(backend_error) => {
                                                return CompileResponse::failure(vec![
                                                    Diagnostic::error(
                                                        backend_error.code.clone(),
                                                        backend_error.message.clone(),
                                                    ),
                                                ]);
                                            }
                                        }
                                    } else {
                                        response = response.with_ir(ir);
                                    }
                                }
                                response
                            }
                            Err(diagnostics) => CompileResponse::failure(diagnostics),
                        }
                    }
                    Err(diagnostics) => CompileResponse::failure(diagnostics),
                }
            }
            Err(diagnostics) => CompileResponse::failure(diagnostics),
        }
    }
}

impl CompilerApi for FrontendCompiler {
    fn check(&self, request: &CompileRequest) -> CompileResponse {
        self.run(request, false)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct JsonRpcRequest {
    jsonrpc: String,
    #[serde(default)]
    id: Option<Value>,
    method: String,
    #[serde(default)]
    params: Option<Value>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct JsonRpcError {
    code: i32,
    message: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct JsonRpcResponse {
    jsonrpc: String,
    id: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    result: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<JsonRpcError>,
}

impl JsonRpcResponse {
    fn success(id: Option<Value>, result: Value) -> Self {
        Self {
            jsonrpc: "2.0".to_owned(),
            id,
            result: Some(result),
            error: None,
        }
    }

    fn error(id: Option<Value>, code: i32, message: impl Into<String>) -> Self {
        Self {
            jsonrpc: "2.0".to_owned(),
            id,
            result: None,
            error: Some(JsonRpcError {
                code,
                message: message.into(),
            }),
        }
    }
}

#[must_use]
pub fn process_json(input: &str) -> String {
    let request = match serde_json::from_str::<JsonRpcRequest>(input) {
        Ok(request) => request,
        Err(error) => {
            return serialize_response(JsonRpcResponse::error(
                None,
                -32700,
                format!("Parse error: {error}"),
            ));
        }
    };

    if request.jsonrpc != "2.0" {
        return serialize_response(JsonRpcResponse::error(
            request.id,
            -32600,
            "Invalid Request: jsonrpc must be \"2.0\".",
        ));
    }

    let response = match request.method.as_str() {
        "initialize" => JsonRpcResponse::success(
            request.id,
            json!({
                "apiVersion": API_VERSION,
                "coreVersion": env!("CARGO_PKG_VERSION"),
                "commands": ["initialize", "check", "check_project", "compile", "compile_project", "ast", "format", "explain", "shutdown"],
                "features": ["resolver", "type-checker", "project-checker", "typed-ir", "project-compile-c"],
                "targets": ["js", "ts", "native", "wasm-web", "wasm-wasi"],
            }),
        ),
        "check" => match parse_compile_request(request.params) {
            Ok(compile_request) => JsonRpcResponse::success(
                request.id,
                serde_json::to_value(FrontendCompiler.check(&compile_request))
                    .unwrap_or(Value::Null),
            ),
            Err(message) => JsonRpcResponse::error(request.id, -32602, message),
        },
        "compile" => match parse_compile_request(request.params) {
            Ok(compile_request) => JsonRpcResponse::success(
                request.id,
                serde_json::to_value(FrontendCompiler.compile(&compile_request))
                    .unwrap_or(Value::Null),
            ),
            Err(message) => JsonRpcResponse::error(request.id, -32602, message),
        },
        "check_project" => match parse_project_check_request(request.params) {
            Ok(project_request) => JsonRpcResponse::success(
                request.id,
                serde_json::to_value(check_project(&project_request)).unwrap_or(Value::Null),
            ),
            Err(message) => JsonRpcResponse::error(request.id, -32602, message),
        },
        "compile_project" => match parse_project_compile_request(request.params) {
            Ok(project_request) => JsonRpcResponse::success(
                request.id,
                serde_json::to_value(compile_project(&project_request)).unwrap_or(Value::Null),
            ),
            Err(message) => JsonRpcResponse::error(request.id, -32602, message),
        },
        "format" => match parse_compile_request(request.params) {
            Ok(compile_request) => {
                let response = match format_source(&compile_request.source.text) {
                    Ok(source) => FormatResponse::success(source),
                    Err(diagnostics) => FormatResponse::failure(diagnostics),
                };
                JsonRpcResponse::success(
                    request.id,
                    serde_json::to_value(response).unwrap_or(Value::Null),
                )
            }
            Err(message) => JsonRpcResponse::error(request.id, -32602, message),
        },
        "shutdown" => JsonRpcResponse::success(request.id, Value::Null),
        "ast" => match parse_compile_request(request.params) {
            Ok(compile_request) => JsonRpcResponse::success(
                request.id,
                serde_json::to_value(ast_for_request(&compile_request)).unwrap_or(Value::Null),
            ),
            Err(message) => JsonRpcResponse::error(request.id, -32602, message),
        },
        "explain" => match parse_compile_request(request.params) {
            Ok(compile_request) => JsonRpcResponse::success(
                request.id,
                serde_json::to_value(explain_for_request(&compile_request)).unwrap_or(Value::Null),
            ),
            Err(message) => JsonRpcResponse::error(request.id, -32602, message),
        },
        _ => JsonRpcResponse::error(request.id, -32601, "Method not found."),
    };

    serialize_response(response)
}

fn ast_for_request(request: &CompileRequest) -> AstResponse {
    match parse(&request.source.text) {
        Ok(module) => AstResponse::success(module),
        Err(diagnostics) => AstResponse::failure(diagnostics),
    }
}

fn explain_for_request(request: &CompileRequest) -> ExplainResponse {
    match parse(&request.source.text) {
        Ok(module) => ExplainResponse::success(infer_module_mode_named(
            &module,
            request.source.path.clone(),
        )),
        Err(diagnostics) => ExplainResponse::failure(diagnostics),
    }
}

fn parse_compile_request(params: Option<Value>) -> Result<CompileRequest, String> {
    let params = params.ok_or_else(|| "Missing params.".to_owned())?;
    let request: CompileRequest =
        serde_json::from_value(params).map_err(|error| format!("Invalid params: {error}"))?;

    if request.api_version != API_VERSION {
        return Err(format!(
            "Unsupported apiVersion '{}'; expected '{}'.",
            request.api_version, API_VERSION
        ));
    }

    Ok(request)
}

fn parse_project_check_request(params: Option<Value>) -> Result<ProjectCheckRequest, String> {
    let params = params.ok_or_else(|| "Missing params.".to_owned())?;
    let request: ProjectCheckRequest =
        serde_json::from_value(params).map_err(|error| format!("Invalid params: {error}"))?;

    if request.api_version != API_VERSION {
        return Err(format!(
            "Unsupported apiVersion '{}'; expected '{}'.",
            request.api_version, API_VERSION
        ));
    }

    if request.sources.is_empty() {
        return Err("check_project requires at least one source unit.".to_owned());
    }

    Ok(request)
}

fn parse_project_compile_request(params: Option<Value>) -> Result<ProjectCompileRequest, String> {
    let params = params.ok_or_else(|| "Missing params.".to_owned())?;
    let request: ProjectCompileRequest =
        serde_json::from_value(params).map_err(|error| format!("Invalid params: {error}"))?;

    if request.api_version != API_VERSION {
        return Err(format!(
            "Unsupported apiVersion '{}'; expected '{}'.",
            request.api_version, API_VERSION
        ));
    }

    if request.sources.is_empty() {
        return Err("compile_project requires at least one source unit.".to_owned());
    }

    Ok(request)
}

fn resolve_project_target(
    importer: &str,
    specifier: &str,
    graph: &ModuleGraph,
    import_map: &BTreeMap<String, String>,
) -> String {
    if let Some(target) = import_map.get(specifier) {
        return target.clone();
    }
    if graph.modules.contains_key(specifier) {
        return specifier.to_owned();
    }

    if let Some(target) = resolve_relative_target(importer, specifier) {
        return target;
    }

    if !specifier.contains('/') && specifier.contains('.') && !specifier.ends_with(".ns") {
        return format!("{}.ns", specifier.replace('.', "/"));
    }

    specifier.to_owned()
}

fn resolve_relative_target(importer: &str, specifier: &str) -> Option<String> {
    if !specifier.starts_with("./") && !specifier.starts_with("../") {
        return None;
    }

    let mut parts: Vec<&str> = importer.split('/').collect();
    parts.pop();
    for component in specifier.split('/') {
        match component {
            "" | "." => {}
            ".." => {
                parts.pop()?;
            }
            other => parts.push(other),
        }
    }

    let mut target = parts.join("/");
    if target.is_empty() {
        return None;
    }
    if !target.ends_with(".ns") {
        target.push_str(".ns");
    }
    Some(target)
}

fn dependency_order(modules: &[ProjectModule], graph: &ModuleGraph) -> Vec<String> {
    fn visit(
        key: &str,
        graph: &ModuleGraph,
        order: &mut Vec<String>,
        visiting: &mut std::collections::BTreeSet<String>,
        visited: &mut std::collections::BTreeSet<String>,
    ) {
        if visited.contains(key) || !visiting.insert(key.to_owned()) {
            return;
        }
        for ((importer, _specifier), target) in &graph.imports {
            if importer == key {
                visit(target, graph, order, visiting, visited);
            }
        }
        visiting.remove(key);
        visited.insert(key.to_owned());
        order.push(key.to_owned());
    }

    let mut order = Vec::new();
    let mut visiting = std::collections::BTreeSet::new();
    let mut visited = std::collections::BTreeSet::new();
    for module in modules {
        visit(&module.key, graph, &mut order, &mut visiting, &mut visited);
    }
    order
}

fn serialize_response(response: JsonRpcResponse) -> String {
    serde_json::to_string(&response).unwrap_or_else(|_| {
        r#"{"jsonrpc":"2.0","id":null,"error":{"code":-32603,"message":"Internal error."}}"#
            .to_owned()
    })
}
