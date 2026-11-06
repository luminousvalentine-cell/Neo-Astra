use std::collections::BTreeMap;

use crate::ast;
use crate::checker::{CheckOptions, CheckResult, check_resolved};
use crate::diagnostic::Diagnostic;
use crate::hir;
use crate::resolver::{ModuleGraph, ModuleResolver, collect_module_specifiers};
use crate::types::{FunctionType, Type};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExportedValue {

    pub name: String,

    pub ty: Type,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ImportedTypeDef {
    Struct {
        name: String,
        type_parameters: Vec<String>,
        fields: ast::StructFields,
    },
    Enum {
        name: String,
        type_parameters: Vec<String>,
        variants: Vec<ast::EnumVariant>,
    },
}

impl ImportedTypeDef {

    #[must_use]
    pub fn name(&self) -> &str {
        match self {
            Self::Struct { name, .. } | Self::Enum { name, .. } => name,
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ModuleTypeSurface {

    pub values: BTreeMap<String, Type>,

    pub type_defs: BTreeMap<String, ImportedTypeDef>,

    pub namespaces: BTreeMap<String, BTreeMap<String, Type>>,
}

impl ModuleTypeSurface {

    #[must_use]
    pub fn get_value(&self, name: &str) -> Option<&Type> {
        self.values.get(name)
    }

    #[must_use]
    pub fn get_type_def(&self, name: &str) -> Option<&ImportedTypeDef> {
        self.type_defs.get(name)
    }
}

#[must_use]
pub fn module_type_surface(result: &CheckResult) -> ModuleTypeSurface {
    module_type_surface_with_surfaces(result, &BTreeMap::new())
}

#[must_use]
pub fn module_type_surface_with_surfaces(
    result: &CheckResult,
    dependencies: &BTreeMap<String, ModuleTypeSurface>,
) -> ModuleTypeSurface {
    let mut values: BTreeMap<String, Type> = BTreeMap::new();
    let mut type_defs: BTreeMap<String, ImportedTypeDef> = BTreeMap::new();
    let mut namespaces: BTreeMap<String, BTreeMap<String, Type>> = BTreeMap::new();

    let item_types: BTreeMap<&str, &Type> = result
        .module
        .items
        .iter()
        .filter_map(|item| item.ty.as_ref().map(|ty| (item.name.as_str(), ty)))
        .collect();

    let item_decls: BTreeMap<&str, &hir::Declaration> = result
        .module
        .items
        .iter()
        .map(|item| (item.name.as_str(), &item.declaration))
        .collect();

    let export_names = result
        .module
        .exports
        .named
        .keys()
        .map(|name| name.as_str())
        .chain(
            result
                .module
                .exports
                .default
                .as_ref()
                .map(|export| export.name.as_str()),
        )
        .collect::<Vec<_>>();

    for exported_name in export_names {
        let export = if exported_name == "default" {
            result.module.exports.default.as_ref()
        } else {
            result.module.exports.named.get(exported_name)
        };

        let Some(export) = export else {
            continue;
        };

        if let hir::ExportTarget::Reexport { module, name } = &export.target
            && let Some(dependency) = dependencies.get(module)
        {
            if export.kind.is_value() {
                values.insert(
                    exported_name.to_owned(),
                    dependency
                        .values
                        .get(name)
                        .cloned()
                        .unwrap_or(Type::Unknown),
                );
            }
            if export.kind.is_type()
                && let Some(definition) = dependency.type_defs.get(name)
            {
                type_defs.insert(exported_name.to_owned(), definition.clone());
            }
            if let Some(members) = dependency.namespaces.get(name) {
                namespaces.insert(exported_name.to_owned(), members.clone());
            }
            continue;
        }

        let local_name = match &export.target {
            hir::ExportTarget::Local(name) => name.as_str(),
            hir::ExportTarget::Reexport { name, .. } => name.as_str(),
            hir::ExportTarget::Namespace { module } => {
                if export.kind.is_value() {
                    values.insert(exported_name.to_owned(), Type::Unknown);
                    if let Some(dependency) = dependencies.get(module) {
                        namespaces.insert(exported_name.to_owned(), dependency.values.clone());
                    }
                }
                continue;
            }
            hir::ExportTarget::Expression => {
                if export.kind.is_value() {
                    values.insert(exported_name.to_owned(), Type::Unknown);
                }
                continue;
            }
        };

        if export.kind.is_value() {
            let ty = item_types
                .get(local_name)
                .copied()
                .cloned()
                .unwrap_or(Type::Unknown);
            values.insert(exported_name.to_owned(), ty);
        }

        if export.kind.is_type()
            && let Some(decl) = item_decls.get(local_name)
        {
            match decl {
                hir::Declaration::Struct(s) => {
                    type_defs.insert(
                        exported_name.to_owned(),
                        ImportedTypeDef::Struct {
                            name: s.name.clone(),
                            type_parameters: s.type_parameters.clone(),
                            fields: s.fields.clone(),
                        },
                    );
                }
                hir::Declaration::Enum(e) => {
                    type_defs.insert(
                        exported_name.to_owned(),
                        ImportedTypeDef::Enum {
                            name: e.name.clone(),
                            type_parameters: e.type_parameters.clone(),
                            variants: e.variants.clone(),
                        },
                    );
                }
                _ => {}
            }
        }
    }

    ModuleTypeSurface {
        values,
        type_defs,
        namespaces,
    }
}

#[derive(Debug, Clone)]
pub struct ProjectModule {

    pub key: String,

    pub source: ast::Module,
}

#[derive(Debug, Clone)]
pub struct GraphCheckResult {

    pub modules: Vec<(String, CheckResult)>,

    pub diagnostics: Vec<(String, Vec<Diagnostic>)>,
}

impl GraphCheckResult {

    #[must_use]
    pub fn is_ok(&self) -> bool {
        self.diagnostics.is_empty()
    }

    #[must_use]
    pub fn get(&self, key: &str) -> Option<&CheckResult> {
        self.modules
            .iter()
            .find_map(|(k, result)| if k == key { Some(result) } else { None })
    }

    #[must_use]
    pub fn all_diagnostics(&self) -> Vec<Diagnostic> {
        self.diagnostics
            .iter()
            .flat_map(|(_, diagnostics)| diagnostics.iter().cloned())
            .collect()
    }
}

pub fn check_module_graph(modules: &[ProjectModule], options: CheckOptions) -> GraphCheckResult {
    let mut graph = ModuleGraph::default();
    for module in modules {
        graph.insert_module(module.key.clone(), module.source.clone());
    }

    for module in modules {
        for specifier in collect_module_specifiers(&module.source) {
            graph.insert_import(
                module.key.clone(),
                specifier.specifier.clone(),
                specifier.specifier,
            );
        }
    }
    check_module_graph_with_imports(modules, &graph, options)
}

pub fn check_module_graph_with_imports(
    modules: &[ProjectModule],
    graph: &ModuleGraph,
    options: CheckOptions,
) -> GraphCheckResult {
    let mut surfaces: BTreeMap<String, ModuleTypeSurface> = BTreeMap::new();
    let mut results: Vec<(String, CheckResult)> = Vec::new();
    let mut all_diagnostics: Vec<(String, Vec<Diagnostic>)> = Vec::new();

    let resolver = ModuleResolver::from_graph(graph);

    for project_module in modules {

        let resolved = match resolver.resolve_named(&project_module.key) {
            Ok(resolved) => resolved,
            Err(diagnostics) => {
                all_diagnostics.push((project_module.key.clone(), diagnostics));
                continue;
            }
        };

        let module_options = CheckOptions {
            module_surfaces: surfaces.clone(),
            ..options.clone()
        };

        match check_resolved(&project_module.source, &resolved, module_options) {
            Ok(result) => {
                let surface = module_type_surface_with_surfaces(&result, &surfaces);
                surfaces.insert(project_module.key.clone(), surface);
                results.push((project_module.key.clone(), result));
            }
            Err(diagnostics) => {
                all_diagnostics.push((project_module.key.clone(), diagnostics));
            }
        }
    }

    GraphCheckResult {
        modules: results,
        diagnostics: all_diagnostics,
    }
}

#[must_use]
pub fn lookup_import_type(
    surfaces: &BTreeMap<String, ModuleTypeSurface>,
    module_key: &str,
    imported_name: &str,
) -> Option<Type> {
    let surface = surfaces.get(module_key)?;
    surface.get_value(imported_name).cloned()
}

#[must_use]
pub fn lookup_import_type_def<'a>(
    surfaces: &'a BTreeMap<String, ModuleTypeSurface>,
    module_key: &str,
    imported_name: &str,
) -> Option<&'a ImportedTypeDef> {
    let surface = surfaces.get(module_key)?;
    surface.get_type_def(imported_name)
}

#[must_use]
pub fn surfaces_for_imports(
    all_surfaces: &BTreeMap<String, ModuleTypeSurface>,
    imports: &[hir::Import],
) -> BTreeMap<String, ModuleTypeSurface> {
    imports
        .iter()
        .filter_map(|import| {
            all_surfaces
                .get(&import.module)
                .map(|surface| (import.module.clone(), surface.clone()))
        })
        .collect()
}

#[must_use]
pub fn as_function_type(ty: &Type) -> Option<&FunctionType> {
    match ty {
        Type::Function(function) => Some(function),
        _ => None,
    }
}
