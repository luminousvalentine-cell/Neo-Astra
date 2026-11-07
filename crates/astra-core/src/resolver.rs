use std::collections::{BTreeMap, BTreeSet};

use crate::ast::{
    self, ExportForm, ImportForm, ItemKind as AstItemKind, Type as AstType, TypeKind, TypePath,
    Visibility,
};
use crate::diagnostic::Diagnostic;
use crate::hir;
use crate::hir::{
    Declaration, Export, ExportKind, ExportSurface, ExportTarget, Import, ImportBinding,
    ImportKind, Item, ItemKind, StarExport, Symbol, SymbolKind, SymbolOrigin, SymbolTable,
};
use crate::span::Span;

const PRIVATE_EXPORT: &str = "NS3001";
const DUPLICATE_EXPORT: &str = "NS3003";
const DUPLICATE_DEFAULT_EXPORT: &str = "NS3004";
const UNKNOWN_EXPORT_TARGET: &str = "NS3005";
const DUPLICATE_IMPORT_BINDING: &str = "NS3006";
const MISSING_IMPORTED_NAME: &str = "NS3007";
const CONFLICTING_REEXPORTS: &str = "NS3008";
const DEFAULT_NOT_VALUE: &str = "NS3009";
const PRIVATE_LEAK: &str = "NS3010";
const DUPLICATE_DECLARATION: &str = "NS3011";
const UNRESOLVED_MODULE: &str = "NS3012";
const MODULE_CYCLE: &str = "NS3013";
const SCRIPT_NAMESPACE_COLLISION: &str = "NS7006";

pub type ResolveResult<T> = Result<T, Vec<Diagnostic>>;

pub trait Resolver {
    fn resolve_module(&self, module: &ast::Module) -> ResolveResult<hir::Module>;
}

#[derive(Debug, Clone, Default)]
pub struct ModuleGraph {
    pub modules: BTreeMap<String, ast::Module>,

    pub imports: BTreeMap<(String, String), String>,
}

impl ModuleGraph {
    pub fn insert_module(&mut self, id: impl Into<String>, module: ast::Module) {
        self.modules.insert(id.into(), module);
    }

    pub fn insert_import(
        &mut self,
        importer: impl Into<String>,
        specifier: impl Into<String>,
        target: impl Into<String>,
    ) {
        self.imports
            .insert((importer.into(), specifier.into()), target.into());
    }
}

#[derive(Debug, Default, Clone)]
pub struct ModuleResolver {
    modules: BTreeMap<String, ast::Module>,
    imports: BTreeMap<(String, String), String>,
    strict_imports: bool,
    identity_imports: bool,
}

impl ModuleResolver {
    #[must_use]
    pub fn standalone() -> Self {
        Self::default()
    }

    #[must_use]
    pub fn from_modules<I>(modules: I) -> Self
    where
        I: IntoIterator<Item = (String, ast::Module)>,
    {
        Self {
            modules: modules.into_iter().collect(),
            imports: BTreeMap::new(),
            strict_imports: true,
            identity_imports: true,
        }
    }

    #[must_use]
    pub fn from_graph(graph: &ModuleGraph) -> Self {
        Self {
            modules: graph.modules.clone(),
            imports: graph.imports.clone(),
            strict_imports: true,
            identity_imports: false,
        }
    }

    pub fn resolve(&self, module: &ast::Module) -> ResolveResult<hir::Module> {
        let key = module_key(module);
        let mut state = ResolutionState::new(self);
        state.stack.push(key.clone());
        let result = state.resolve_ast(&key, module);
        state.stack.pop();
        result
    }

    pub fn resolve_named(&self, name: &str) -> ResolveResult<hir::Module> {
        let Some(module) = self.modules.get(name).cloned() else {
            return Err(vec![unresolved_module(name, None)]);
        };

        let mut state = ResolutionState::new(self);
        state.stack.push(name.to_owned());
        let result = state.resolve_ast(name, &module);
        state.stack.pop();
        result
    }

    pub fn resolve_root(&self, id: &str) -> ResolveResult<hir::Module> {
        self.resolve_named(id)
    }
}

impl Resolver for ModuleResolver {
    fn resolve_module(&self, module: &ast::Module) -> ResolveResult<hir::Module> {
        self.resolve(module)
    }
}

pub fn resolve_module(module: &ast::Module) -> ResolveResult<hir::Module> {
    ModuleResolver::standalone().resolve_module(module)
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModuleSpecifier {
    pub specifier: String,

    pub span: Span,
}

#[must_use]
pub fn collect_module_specifiers(module: &ast::Module) -> Vec<ModuleSpecifier> {
    let mut specifiers = Vec::new();
    for item in &module.items {
        match &item.kind {
            AstItemKind::Import(import) => {
                let specifier = match &import.form {
                    ImportForm::Path { path, .. } => Some(path.join(".")),
                    ImportForm::SideEffect { specifier }
                    | ImportForm::Namespace { specifier, .. }
                    | ImportForm::Named { specifier, .. }
                    | ImportForm::Default { specifier, .. }
                    | ImportForm::DefaultNamed { specifier, .. } => Some(specifier.clone()),
                };
                if let Some(specifier) = specifier {
                    specifiers.push(ModuleSpecifier {
                        specifier,
                        span: import.span,
                    });
                }
            }
            AstItemKind::Export(export) => {
                let specifier = match &export.form {
                    ExportForm::List { from, .. } => from.clone(),
                    ExportForm::All { from, .. } => Some(from.clone()),
                    ExportForm::DefaultValue(_) | ExportForm::DefaultFunction(_) => None,
                };
                if let Some(specifier) = specifier {
                    specifiers.push(ModuleSpecifier {
                        specifier,
                        span: export.span,
                    });
                }
            }
            _ => {}
        }
    }
    specifiers
}

fn module_key(module: &ast::Module) -> String {
    if module.name.is_empty() {
        "<root>".to_owned()
    } else {
        module.name.join(".")
    }
}

fn unresolved_module(name: &str, span: Option<Span>) -> Diagnostic {
    let diagnostic = Diagnostic::error(
        UNRESOLVED_MODULE,
        format!("Imported module '{name}' could not be resolved."),
    )
    .with_hint("Add the module to the project module graph or fix the import specifier.");
    if let Some(span) = span {
        diagnostic.with_span(span)
    } else {
        diagnostic
    }
}

struct ResolutionState<'a> {
    resolver: &'a ModuleResolver,
    cache: BTreeMap<String, ResolveResult<hir::Module>>,
    stack: Vec<String>,
    reported: BTreeSet<String>,
}

impl<'a> ResolutionState<'a> {
    fn new(resolver: &'a ModuleResolver) -> Self {
        Self {
            resolver,
            cache: BTreeMap::new(),
            stack: Vec::new(),
            reported: BTreeSet::new(),
        }
    }

    fn resolve_ast(&mut self, key: &str, module: &ast::Module) -> ResolveResult<hir::Module> {
        let mut decls = Vec::new();
        let mut imports = Vec::new();
        let mut exports = Vec::new();
        collect_items(&module.items, &mut decls, &mut imports, &mut exports);

        let mut pass = ModulePass {
            state: self,
            key: key.to_owned(),
            module,
            diagnostics: Vec::new(),
            items: Vec::new(),
            resolved_imports: Vec::new(),
            symbols: SymbolTable::default(),
            resolved_exports: ExportSurface::default(),
            decls,
            imports_ast: imports,
            exports_ast: exports,
            declaration_bindings: BTreeMap::new(),
            import_bindings: BTreeMap::new(),
            default_function_names: BTreeSet::new(),
            script_declarations: BTreeSet::new(),
            pending_stars: Vec::new(),
        };

        pass.run(key)
    }

    fn resolve_named(&mut self, name: &str, span: Option<Span>) -> ResolveResult<hir::Module> {
        if let Some(cached) = self.cache.get(name) {
            return cached.clone();
        }

        if self.stack.iter().any(|entry| entry == name) {
            let diagnostic = Diagnostic::error(
                MODULE_CYCLE,
                format!("Cyclic module import involving '{name}'."),
            )
            .with_hint("Break the named import or re-export cycle.");
            let diagnostic = if let Some(span) = span {
                diagnostic.with_span(span)
            } else {
                diagnostic
            };
            return Err(vec![diagnostic]);
        }

        let Some(module) = self.resolver.modules.get(name).cloned() else {
            return Err(vec![unresolved_module(name, span)]);
        };

        self.stack.push(name.to_owned());
        let result = self.resolve_ast(name, &module);
        self.stack.pop();
        self.cache.insert(name.to_owned(), result.clone());
        result
    }
}

#[derive(Debug, Clone, Copy)]
enum DeclRef<'a> {
    Item(&'a ast::Item),
    DefaultFunction(&'a ast::FunctionDecl),
}

impl<'a> DeclRef<'a> {
    fn name(self) -> &'a str {
        match self {
            Self::Item(item) => match &item.kind {
                AstItemKind::Function(declaration) => &declaration.name,
                AstItemKind::Struct(declaration) => &declaration.name,
                AstItemKind::Enum(declaration) => &declaration.name,
                AstItemKind::Constant(declaration) => &declaration.name,
                AstItemKind::TypeAlias(declaration) => &declaration.name,
                AstItemKind::ForeignModule(declaration) => &declaration.name,
                AstItemKind::ScriptBlock(declaration) => &declaration.name,
                AstItemKind::Import(_) | AstItemKind::Export(_) | AstItemKind::ScopedItems(_) => "",
            },
            Self::DefaultFunction(function) => &function.name,
        }
    }

    fn span(self) -> Span {
        match self {
            Self::Item(item) => item.span,
            Self::DefaultFunction(function) => function.span,
        }
    }

    fn visibility(self) -> Visibility {
        match self {
            Self::Item(item) => item.visibility,
            Self::DefaultFunction(_) => Visibility::Normal,
        }
    }

    fn exported(self) -> bool {
        match self {
            Self::Item(item) => item.exported,
            Self::DefaultFunction(_) => true,
        }
    }

    fn is_default_function(self) -> bool {
        matches!(self, Self::DefaultFunction(_))
    }

    fn declaration(self) -> Declaration {
        match self {
            Self::Item(item) => match &item.kind {
                AstItemKind::Function(declaration) => Declaration::Function(declaration.clone()),
                AstItemKind::Struct(declaration) => Declaration::Struct(declaration.clone()),
                AstItemKind::Enum(declaration) => Declaration::Enum(declaration.clone()),
                AstItemKind::Constant(declaration) => Declaration::Constant(declaration.clone()),
                AstItemKind::TypeAlias(declaration) => Declaration::TypeAlias(declaration.clone()),
                AstItemKind::ForeignModule(declaration) => {
                    Declaration::ForeignModule(declaration.clone())
                }
                AstItemKind::ScriptBlock(declaration) => {
                    Declaration::ScriptBlock(declaration.clone())
                }
                AstItemKind::Import(_) | AstItemKind::Export(_) | AstItemKind::ScopedItems(_) => {
                    unreachable!("only declarations are collected as DeclRef")
                }
            },
            Self::DefaultFunction(function) => Declaration::Function(function.clone()),
        }
    }
}

fn collect_items<'a>(
    items: &'a [ast::Item],
    decls: &mut Vec<DeclRef<'a>>,
    imports: &mut Vec<&'a ast::ImportDecl>,
    exports: &mut Vec<&'a ast::ExportDecl>,
) {
    for item in items {
        match &item.kind {
            AstItemKind::Import(import) => imports.push(import),
            AstItemKind::Export(export) => {
                if let ExportForm::DefaultFunction(function) = &export.form {
                    decls.push(DeclRef::DefaultFunction(function));
                }
                exports.push(export);
            }
            AstItemKind::ScopedItems(scoped) => {
                collect_items(&scoped.items, decls, imports, exports);
            }
            _ => decls.push(DeclRef::Item(item)),
        }
    }
}

struct PendingStar {
    module: String,
    target: TargetSurface,
    span: Span,
}

#[derive(Debug)]
enum TargetSurface {
    Resolved(ExportSurface),
    Opaque,
    Missing,
}

struct LocalBinding {
    kind: ExportKind,
    visibility: Visibility,
}

struct ModulePass<'m, 's, 'r> {
    state: &'s mut ResolutionState<'r>,
    key: String,
    module: &'m ast::Module,
    diagnostics: Vec<Diagnostic>,
    items: Vec<Item>,
    resolved_imports: Vec<Import>,
    symbols: SymbolTable,
    resolved_exports: ExportSurface,
    decls: Vec<DeclRef<'m>>,
    imports_ast: Vec<&'m ast::ImportDecl>,
    exports_ast: Vec<&'m ast::ExportDecl>,
    declaration_bindings: BTreeMap<String, Span>,
    import_bindings: BTreeMap<String, Span>,
    default_function_names: BTreeSet<String>,
    script_declarations: BTreeSet<String>,
    pending_stars: Vec<PendingStar>,
}

impl<'m, 's, 'r> ModulePass<'m, 's, 'r> {
    fn run(&mut self, _key: &str) -> ResolveResult<hir::Module> {
        for declaration in self.decls.clone() {
            self.add_declaration(declaration);
        }

        for import in self.imports_ast.clone() {
            self.process_import(import);
        }

        self.add_declaration_exports();

        for export in self.exports_ast.clone() {
            self.process_export(export);
        }

        self.expand_star_exports();
        self.check_private_leaks();

        if !self.diagnostics.is_empty() {
            return Err(std::mem::take(&mut self.diagnostics));
        }

        Ok(hir::Module {
            name: self.module.name.join("."),
            items: std::mem::take(&mut self.items),
            imports: std::mem::take(&mut self.resolved_imports),
            symbols: std::mem::take(&mut self.symbols),
            exports: std::mem::take(&mut self.resolved_exports),
        })
    }

    fn error(&mut self, code: &str, message: impl Into<String>, span: Span, hint: &str) {
        self.diagnostics.push(
            Diagnostic::error(code, message)
                .with_span(span)
                .with_hint(hint),
        );
    }

    fn add_declaration(&mut self, declaration: DeclRef<'m>) {
        let name = declaration.name().to_owned();
        if name.is_empty() {
            return;
        }

        let span = declaration.span();
        let kind = declaration.declaration().kind();
        let is_script = kind == ItemKind::ScriptBlock;
        if let Some(previous) = self.declaration_bindings.get(&name) {
            let collision_with_script = is_script || self.script_declarations.contains(&name);
            let code = if collision_with_script {
                SCRIPT_NAMESPACE_COLLISION
            } else {
                DUPLICATE_DECLARATION
            };
            self.error(
                code,
                format!("Duplicate declaration '{name}'."),
                span,
                &format!(
                    "The first declaration starts at byte {}. Rename one of the declarations.",
                    previous.start
                ),
            );
            return;
        }

        if is_script {
            self.script_declarations.insert(name.clone());
        }

        self.declaration_bindings.insert(name.clone(), span);

        let visibility = declaration.visibility();
        let exported = declaration.exported() || visibility == Visibility::Public;
        let symbol = Symbol {
            name: name.clone(),
            kind: symbol_kind(kind),
            visibility,
            exported,
            span,
            origin: SymbolOrigin::Local,
        };

        match kind {
            ItemKind::Function | ItemKind::Constant => {
                self.symbols.values.insert(name.clone(), symbol);
            }
            ItemKind::Struct | ItemKind::Enum => {
                self.symbols.values.insert(name.clone(), symbol.clone());
                self.symbols.types.insert(name.clone(), symbol);
            }
            ItemKind::TypeAlias => {
                self.symbols.types.insert(name.clone(), symbol);
            }
            ItemKind::ForeignModule | ItemKind::ScriptBlock => {
                self.symbols.namespaces.insert(name.clone(), symbol);
            }
        }

        if declaration.is_default_function() {
            self.default_function_names.insert(name.clone());
        }

        self.items.push(Item {
            name,
            kind,
            ty: None,
            span,
            visibility,
            exported,
            declaration: declaration.declaration(),
        });
    }

    fn add_declaration_exports(&mut self) {
        let exports: Vec<(String, ExportKind, Span)> = self
            .items
            .iter()
            .filter(|item| !self.default_function_names.contains(&item.name))
            .filter(|item| item.visibility == Visibility::Public || item.exported)
            .map(|item| (item.name.clone(), item.kind.export_kind(), item.span))
            .collect();

        for (name, kind, span) in exports {
            self.add_named_export(name.clone(), kind, ExportTarget::Local(name), span);
        }
    }

    fn process_import(&mut self, import: &'m ast::ImportDecl) {
        match &import.form {
            ImportForm::Path { path, alias } => {
                let specifier = path.join(".");
                let Some(module) = self.canonical_target(&specifier, import.span) else {
                    return;
                };
                let alias = alias
                    .clone()
                    .or_else(|| path.last().cloned())
                    .unwrap_or_default();
                let available = self.target_exists(&module, import.span);
                if available && !alias.is_empty() {
                    self.add_import_namespace(&module, &alias, import.span);
                }
                self.resolved_imports.push(Import {
                    module,
                    kind: ImportKind::Namespace { alias },
                    span: import.span,
                });
            }
            ImportForm::SideEffect { specifier } => {
                let Some(module) = self.canonical_target(specifier, import.span) else {
                    return;
                };
                self.target_exists(&module, import.span);
                self.resolved_imports.push(Import {
                    module,
                    kind: ImportKind::SideEffect,
                    span: import.span,
                });
            }
            ImportForm::Namespace { alias, specifier } => {
                let Some(module) = self.canonical_target(specifier, import.span) else {
                    return;
                };
                let available = self.target_exists(&module, import.span);
                if available && !alias.is_empty() {
                    self.add_import_namespace(&module, alias, import.span);
                }
                self.resolved_imports.push(Import {
                    module,
                    kind: ImportKind::Namespace {
                        alias: alias.clone(),
                    },
                    span: import.span,
                });
            }
            ImportForm::Named {
                specifiers,
                specifier,
            } => {
                let Some(module) = self.canonical_target(specifier, import.span) else {
                    return;
                };
                let target = self.target_surface(&module, import.span);
                let mut bindings = Vec::new();
                for binding_ast in specifiers {
                    if let Some(binding) = self.import_named(&target, binding_ast, &module) {
                        bindings.push(binding);
                    }
                }
                self.resolved_imports.push(Import {
                    module,
                    kind: ImportKind::Named { bindings },
                    span: import.span,
                });
            }
            ImportForm::Default { alias, specifier } => {
                let Some(module) = self.canonical_target(specifier, import.span) else {
                    return;
                };
                let target = self.target_surface(&module, import.span);
                let _ = self.import_default(&target, &module, alias, import.span);
                self.resolved_imports.push(Import {
                    module,
                    kind: ImportKind::Default {
                        local: alias.clone(),
                    },
                    span: import.span,
                });
            }
            ImportForm::DefaultNamed {
                default,
                specifiers,
                specifier,
            } => {
                let Some(module) = self.canonical_target(specifier, import.span) else {
                    return;
                };
                let target = self.target_surface(&module, import.span);
                let _ = self.import_default(&target, &module, default, import.span);
                let mut bindings = Vec::new();
                for binding_ast in specifiers {
                    if let Some(binding) = self.import_named(&target, binding_ast, &module) {
                        bindings.push(binding);
                    }
                }
                self.resolved_imports.push(Import {
                    module,
                    kind: ImportKind::DefaultNamed {
                        default: default.clone(),
                        bindings,
                    },
                    span: import.span,
                });
            }
        }
    }

    fn canonical_target(&mut self, specifier: &str, span: Span) -> Option<String> {
        if let Some(target) = self
            .state
            .resolver
            .imports
            .get(&(self.key.clone(), specifier.to_owned()))
        {
            return Some(target.clone());
        }

        if !self.state.resolver.strict_imports {
            return Some(specifier.to_owned());
        }

        if self.state.resolver.identity_imports
            && self.state.resolver.modules.contains_key(specifier)
        {
            return Some(specifier.to_owned());
        }

        self.diagnostics
            .push(unresolved_module(specifier, Some(span)));
        None
    }

    fn target_exists(&mut self, module: &str, span: Span) -> bool {
        if self.state.resolver.modules.contains_key(module) {
            return true;
        }

        if self.state.resolver.strict_imports {
            self.diagnostics.push(unresolved_module(module, Some(span)));
            return false;
        }

        true
    }

    fn target_surface(&mut self, module: &str, span: Span) -> TargetSurface {
        if !self.state.resolver.modules.contains_key(module) {
            if self.state.resolver.strict_imports {
                self.diagnostics.push(unresolved_module(module, Some(span)));
                return TargetSurface::Missing;
            }
            return TargetSurface::Opaque;
        }

        match self.state.resolve_named(module, Some(span)) {
            Ok(resolved) => TargetSurface::Resolved(resolved.exports),
            Err(diagnostics) => {
                if self.state.reported.insert(module.to_owned()) {
                    self.diagnostics.extend(diagnostics);
                }
                TargetSurface::Missing
            }
        }
    }

    fn add_import_namespace(&mut self, module: &str, alias: &str, span: Span) -> bool {
        if !self.claim_import_binding(alias, span) {
            return false;
        }

        self.symbols.namespaces.insert(
            alias.to_owned(),
            Symbol {
                name: alias.to_owned(),
                kind: SymbolKind::ImportedNamespace,
                visibility: Visibility::Normal,
                exported: false,
                span,
                origin: SymbolOrigin::Imported {
                    module: module.to_owned(),
                    name: "*".to_owned(),
                },
            },
        );
        true
    }

    fn import_named(
        &mut self,
        target: &TargetSurface,
        specifier: &ast::ImportSpecifier,
        module: &str,
    ) -> Option<ImportBinding> {
        if specifier.imported == "default" && specifier.local == "default" {
            self.error(
                DUPLICATE_IMPORT_BINDING,
                "The default import must be aliased to a local name.",
                specifier.span,
                "Write `default as localName`.",
            );
            return None;
        }

        let kind = match target {
            TargetSurface::Missing => return None,
            TargetSurface::Opaque => {
                if specifier.imported == "default" {
                    ExportKind::Value
                } else {
                    ExportKind::ValueAndType
                }
            }
            TargetSurface::Resolved(exports) => {
                if specifier.imported == "default" {
                    let Some(export) = &exports.default else {
                        self.error(
                            MISSING_IMPORTED_NAME,
                            format!("Module '{module}' has no default export."),
                            specifier.span,
                            "Import a named export or add a default export.",
                        );
                        return None;
                    };
                    if !export.kind.is_value() {
                        self.error(
                            MISSING_IMPORTED_NAME,
                            format!("Module '{module}' default export is not a value."),
                            specifier.span,
                            "Default exports must be values.",
                        );
                        return None;
                    }
                    export.kind
                } else {
                    let Some(export) = exports.get_named(&specifier.imported) else {
                        self.error(
                            MISSING_IMPORTED_NAME,
                            format!(
                                "Module '{module}' does not export '{}'.",
                                specifier.imported
                            ),
                            specifier.span,
                            "Check the target module's export surface.",
                        );
                        return None;
                    };
                    export.kind
                }
            }
        };

        self.add_import_binding(
            module,
            &specifier.imported,
            &specifier.local,
            kind,
            specifier.span,
        )
    }

    fn import_default(
        &mut self,
        target: &TargetSurface,
        module: &str,
        local: &str,
        span: Span,
    ) -> Option<ImportBinding> {
        if local.is_empty() {
            return None;
        }

        let kind = match target {
            TargetSurface::Missing => return None,
            TargetSurface::Opaque => ExportKind::Value,
            TargetSurface::Resolved(exports) => {
                let Some(export) = &exports.default else {
                    self.error(
                        MISSING_IMPORTED_NAME,
                        format!("Module '{module}' has no default export."),
                        span,
                        "Import a named export or add a default export.",
                    );
                    return None;
                };
                if !export.kind.is_value() {
                    self.error(
                        MISSING_IMPORTED_NAME,
                        format!("Module '{module}' default export is not a value."),
                        span,
                        "Default exports must be values.",
                    );
                    return None;
                }
                export.kind
            }
        };

        self.add_import_binding(module, "default", local, kind, span)
    }

    fn add_import_binding(
        &mut self,
        module: &str,
        imported: &str,
        local: &str,
        kind: ExportKind,
        span: Span,
    ) -> Option<ImportBinding> {
        if local.is_empty() || !self.claim_import_binding(local, span) {
            return None;
        }

        let origin = SymbolOrigin::Imported {
            module: module.to_owned(),
            name: imported.to_owned(),
        };
        let symbol = |kind| Symbol {
            name: local.to_owned(),
            kind,
            visibility: Visibility::Normal,
            exported: false,
            span,
            origin: origin.clone(),
        };

        match kind {
            ExportKind::Value => {
                self.symbols
                    .values
                    .insert(local.to_owned(), symbol(SymbolKind::ImportedValue));
            }
            ExportKind::Type => {
                self.symbols
                    .types
                    .insert(local.to_owned(), symbol(SymbolKind::ImportedType));
            }
            ExportKind::ValueAndType => {
                self.symbols
                    .values
                    .insert(local.to_owned(), symbol(SymbolKind::ImportedValue));
                self.symbols
                    .types
                    .insert(local.to_owned(), symbol(SymbolKind::ImportedType));
            }
            ExportKind::Namespace => {
                self.symbols
                    .namespaces
                    .insert(local.to_owned(), symbol(SymbolKind::ImportedNamespace));
            }
        }

        Some(ImportBinding {
            imported: imported.to_owned(),
            local: local.to_owned(),
            span,
        })
    }

    fn claim_import_binding(&mut self, name: &str, span: Span) -> bool {
        if self.symbols.contains_name(name) || self.import_bindings.contains_key(name) {
            self.error(
                DUPLICATE_IMPORT_BINDING,
                format!("Duplicate import binding '{name}'."),
                span,
                "Rename one of the imported bindings.",
            );
            return false;
        }

        self.import_bindings.insert(name.to_owned(), span);
        true
    }

    fn process_export(&mut self, export: &'m ast::ExportDecl) {
        match &export.form {
            ExportForm::List { specifiers, from } => {
                if let Some(specifier) = from {
                    let Some(module) = self.canonical_target(specifier, export.span) else {
                        return;
                    };
                    let target = self.target_surface(&module, export.span);
                    for specifier in specifiers {
                        self.export_from(&target, &module, specifier);
                    }
                } else {
                    for specifier in specifiers {
                        self.export_local(specifier);
                    }
                }
            }
            ExportForm::All { alias, from } => {
                let Some(module) = self.canonical_target(from, export.span) else {
                    return;
                };
                if let Some(alias) = alias {
                    if !self.target_exists(&module, export.span) {
                        return;
                    }
                    let target = ExportTarget::Namespace {
                        module: module.clone(),
                    };
                    if alias == "default" {
                        self.add_default_export(Export {
                            name: "default".to_owned(),
                            kind: ExportKind::Namespace,
                            target,
                            span: export.span,
                        });
                    } else {
                        self.add_named_export(
                            alias.clone(),
                            ExportKind::Namespace,
                            target,
                            export.span,
                        );
                    }
                } else {
                    let target = self.target_surface(&module, export.span);
                    self.resolved_exports.stars.push(StarExport {
                        module: module.clone(),
                        span: export.span,
                    });
                    self.pending_stars.push(PendingStar {
                        module,
                        target,
                        span: export.span,
                    });
                }
            }
            ExportForm::DefaultValue(expression) => {
                let target = if let ast::ExprKind::Identifier(name) = &expression.kind {
                    let Some(binding) = self.lookup_local(name) else {
                        self.error(
                            UNKNOWN_EXPORT_TARGET,
                            format!("Unknown default export target '{name}'."),
                            expression.span,
                            "Export a local value or use an expression.",
                        );
                        return;
                    };
                    if binding.visibility == Visibility::Private {
                        self.error(
                            PRIVATE_EXPORT,
                            format!("Private declaration '{name}' cannot be exported."),
                            expression.span,
                            "Remove `private` or export a different value.",
                        );
                        return;
                    }
                    if !binding.kind.is_value() {
                        self.error(
                            DEFAULT_NOT_VALUE,
                            "The default export target is not a value.",
                            expression.span,
                            "Export a function, constant, or struct constructor.",
                        );
                        return;
                    }
                    ExportTarget::Local(name.clone())
                } else {
                    ExportTarget::Expression
                };

                self.add_default_export(Export {
                    name: "default".to_owned(),
                    kind: ExportKind::Value,
                    target,
                    span: export.span,
                });
            }
            ExportForm::DefaultFunction(function) => {
                let target = if function.name.is_empty() {
                    ExportTarget::Expression
                } else {
                    ExportTarget::Local(function.name.clone())
                };
                self.add_default_export(Export {
                    name: "default".to_owned(),
                    kind: ExportKind::Value,
                    target,
                    span: function.span,
                });
            }
        }
    }

    fn export_local(&mut self, specifier: &ast::ExportSpecifier) {
        if specifier.local == "default" {
            self.error(
                UNKNOWN_EXPORT_TARGET,
                "Cannot export `default` without a source module.",
                specifier.span,
                "Use `export { local as default }` or add `from \"module\"`.",
            );
            return;
        }

        let Some(binding) = self.lookup_local(&specifier.local) else {
            self.error(
                UNKNOWN_EXPORT_TARGET,
                format!("Unknown export target '{}'.", specifier.local),
                specifier.span,
                "Export a local declaration or imported binding.",
            );
            return;
        };

        if binding.visibility == Visibility::Private {
            self.error(
                PRIVATE_EXPORT,
                format!(
                    "Private declaration '{}' cannot be exported.",
                    specifier.local
                ),
                specifier.span,
                "Remove `private` or export a different declaration.",
            );
            return;
        }

        if specifier.exported == "default" {
            if !binding.kind.is_value() {
                self.error(
                    DEFAULT_NOT_VALUE,
                    "The default export target is not a value.",
                    specifier.span,
                    "Default exports must be functions, constants, or instances.",
                );
                return;
            }
            self.add_default_export(Export {
                name: "default".to_owned(),
                kind: binding.kind,
                target: ExportTarget::Local(specifier.local.clone()),
                span: specifier.span,
            });
        } else {
            self.add_named_export(
                specifier.exported.clone(),
                binding.kind,
                ExportTarget::Local(specifier.local.clone()),
                specifier.span,
            );
        }
    }

    fn export_from(
        &mut self,
        target: &TargetSurface,
        module: &str,
        specifier: &ast::ExportSpecifier,
    ) {
        let kind = match target {
            TargetSurface::Missing => return,
            TargetSurface::Opaque => {
                if specifier.local == "default" {
                    ExportKind::Value
                } else {
                    ExportKind::ValueAndType
                }
            }
            TargetSurface::Resolved(exports) => {
                if specifier.local == "default" {
                    let Some(export) = &exports.default else {
                        self.error(
                            MISSING_IMPORTED_NAME,
                            format!("Module '{module}' has no default export."),
                            specifier.span,
                            "Re-export a named export or add a default export.",
                        );
                        return;
                    };
                    export.kind
                } else {
                    let Some(export) = exports.get_named(&specifier.local) else {
                        self.error(
                            MISSING_IMPORTED_NAME,
                            format!("Module '{module}' does not export '{}'.", specifier.local),
                            specifier.span,
                            "Check the target module's export surface.",
                        );
                        return;
                    };
                    export.kind
                }
            }
        };

        let target = ExportTarget::Reexport {
            module: module.to_owned(),
            name: specifier.local.clone(),
        };

        if specifier.exported == "default" {
            if !kind.is_value() {
                self.error(
                    DEFAULT_NOT_VALUE,
                    "The default export target is not a value.",
                    specifier.span,
                    "Default exports must be values.",
                );
                return;
            }
            self.add_default_export(Export {
                name: "default".to_owned(),
                kind,
                target,
                span: specifier.span,
            });
        } else {
            self.add_named_export(specifier.exported.clone(), kind, target, specifier.span);
        }
    }

    fn lookup_local(&self, name: &str) -> Option<LocalBinding> {
        let value = self.symbols.values.get(name);
        let ty = self.symbols.types.get(name);
        let namespace = self.symbols.namespaces.get(name);

        let kind = match (value, ty, namespace) {
            (Some(_), Some(_), _) => ExportKind::ValueAndType,
            (Some(symbol), None, None) => symbol.kind.export_kind(),
            (None, Some(symbol), None) => symbol.kind.export_kind(),
            (None, None, Some(symbol)) => symbol.kind.export_kind(),
            (None, None, None) => return None,
            _ => return None,
        };

        let visibility = value
            .into_iter()
            .chain(ty)
            .chain(namespace)
            .map(|symbol| symbol.visibility)
            .find(|visibility| *visibility == Visibility::Private)
            .or_else(|| {
                value
                    .into_iter()
                    .chain(ty)
                    .chain(namespace)
                    .map(|symbol| symbol.visibility)
                    .find(|visibility| *visibility == Visibility::Public)
            })
            .unwrap_or(Visibility::Normal);

        Some(LocalBinding { kind, visibility })
    }

    fn add_named_export(
        &mut self,
        name: String,
        kind: ExportKind,
        target: ExportTarget,
        span: Span,
    ) {
        if name.is_empty() {
            return;
        }

        if name == "default" {
            if !kind.is_value() {
                self.error(
                    DEFAULT_NOT_VALUE,
                    "The default export target is not a value.",
                    span,
                    "Default exports must be values.",
                );
                return;
            }
            self.add_default_export(Export {
                name,
                kind,
                target,
                span,
            });
            return;
        }

        if let Some(existing) = self.resolved_exports.named.get(&name) {
            if existing.target == target && existing.kind == kind {
                return;
            }

            self.error(
                DUPLICATE_EXPORT,
                format!("Duplicate export name '{name}'."),
                span,
                "Rename one export or remove the duplicate.",
            );
            return;
        }

        self.resolved_exports.named.insert(
            name.clone(),
            Export {
                name,
                kind,
                target,
                span,
            },
        );
    }

    fn add_default_export(&mut self, export: Export) {
        if self.resolved_exports.default.is_some() {
            self.error(
                DUPLICATE_DEFAULT_EXPORT,
                "A module may have only one default export.",
                export.span,
                "Remove the extra default export.",
            );
            return;
        }

        if !export.kind.is_value() {
            self.error(
                DEFAULT_NOT_VALUE,
                "The default export target is not a value.",
                export.span,
                "Default exports must be values.",
            );
            return;
        }

        let mut export = export;
        if export.kind == ExportKind::ValueAndType {
            export.kind = ExportKind::Value;
        }
        self.resolved_exports.default = Some(export);
    }

    fn expand_star_exports(&mut self) {
        let pending = std::mem::take(&mut self.pending_stars);
        let mut origins: BTreeMap<String, String> = BTreeMap::new();

        for star in pending {
            let TargetSurface::Resolved(exports) = star.target else {
                continue;
            };

            for (name, source) in exports.named {
                if let Some(previous) = origins.get(&name) {
                    if previous != &star.module {
                        self.error(
                            CONFLICTING_REEXPORTS,
                            format!("Re-exported name '{name}' is provided by multiple modules."),
                            star.span,
                            "Export the name explicitly from one module.",
                        );
                    }
                    continue;
                }

                if self.resolved_exports.named.contains_key(&name) {
                    continue;
                }

                origins.insert(name.clone(), star.module.clone());
                self.resolved_exports.named.insert(
                    name.clone(),
                    Export {
                        name: name.clone(),
                        kind: source.kind,
                        target: ExportTarget::Reexport {
                            module: star.module.clone(),
                            name,
                        },
                        span: star.span,
                    },
                );
            }
        }
    }

    fn check_private_leaks(&mut self) {
        let mut exported_locals = BTreeSet::new();
        for export in self
            .resolved_exports
            .named
            .values()
            .chain(self.resolved_exports.default.iter())
        {
            if let ExportTarget::Local(name) = &export.target {
                exported_locals.insert(name.clone());
            }
        }

        for item in self.items.clone() {
            let is_public = item.visibility == Visibility::Public
                || item.exported
                || exported_locals.contains(&item.name);
            if !is_public || item.visibility == Visibility::Private {
                continue;
            }

            self.check_declaration_leaks(&item.name, &item.declaration);
        }
    }

    fn check_declaration_leaks(&mut self, owner: &str, declaration: &Declaration) {
        match declaration {
            Declaration::Function(function) => {
                let generics = generic_names(&function.type_parameters);
                for parameter in &function.parameters {
                    self.check_type_leak(owner, &parameter.ty, &generics);
                }
                if let Some(return_type) = &function.return_type {
                    self.check_type_leak(owner, return_type, &generics);
                }
                for error in &function.error_set {
                    self.check_type_path_leak(owner, error, &generics);
                }
            }
            Declaration::Struct(declaration) => {
                let generics = generic_names(&declaration.type_parameters);
                match &declaration.fields {
                    ast::StructFields::Named(fields) => {
                        for field in fields {
                            self.check_type_leak(owner, &field.ty, &generics);
                        }
                    }
                    ast::StructFields::Tuple(fields) => {
                        for field in fields {
                            self.check_type_leak(owner, field, &generics);
                        }
                    }
                }
            }
            Declaration::Enum(declaration) => {
                let generics = generic_names(&declaration.type_parameters);
                for variant in &declaration.variants {
                    if let Some(fields) = &variant.fields {
                        for field in fields {
                            self.check_type_leak(owner, &field.ty, &generics);
                        }
                    }
                }
            }
            Declaration::Constant(constant) => {
                self.check_type_leak(owner, &constant.ty, &BTreeSet::new());
            }
            Declaration::TypeAlias(alias) => {
                let generics = generic_names(&alias.type_parameters);
                self.check_type_leak(owner, &alias.ty, &generics);
            }
            Declaration::ForeignModule(module) => {
                for function in &module.functions {
                    for parameter in &function.parameters {
                        self.check_type_leak(owner, &parameter.ty, &BTreeSet::new());
                    }
                    if let Some(return_type) = &function.return_type {
                        self.check_type_leak(owner, return_type, &BTreeSet::new());
                    }
                    for error in &function.error_set {
                        self.check_type_path_leak(owner, error, &BTreeSet::new());
                    }
                }
            }
            Declaration::ScriptBlock(script) => {
                for export in &script.exports {
                    for parameter in &export.parameters {
                        self.check_type_leak(owner, &parameter.ty, &BTreeSet::new());
                    }
                    if let Some(return_type) = &export.return_type {
                        self.check_type_leak(owner, return_type, &BTreeSet::new());
                    }
                    for error in &export.error_set {
                        self.check_type_path_leak(owner, error, &BTreeSet::new());
                    }
                }
            }
        }
    }

    fn check_type_leak(&mut self, owner: &str, ty: &AstType, generics: &BTreeSet<String>) {
        match &ty.kind {
            TypeKind::Path(path) => self.check_type_path_leak(owner, path, generics),
            TypeKind::Array(element) | TypeKind::FixedArray(element, _) => {
                self.check_type_leak(owner, element, generics);
            }
            TypeKind::Map(key, value) => {
                self.check_type_leak(owner, key, generics);
                self.check_type_leak(owner, value, generics);
            }
            TypeKind::Tuple(elements) => {
                for element in elements {
                    self.check_type_leak(owner, element, generics);
                }
            }
            TypeKind::Function {
                parameters,
                return_type,
                error_set,
            } => {
                for parameter in parameters {
                    self.check_type_leak(owner, parameter, generics);
                }
                if let Some(return_type) = return_type {
                    self.check_type_leak(owner, return_type, generics);
                }
                for error in error_set {
                    self.check_type_path_leak(owner, error, generics);
                }
            }
            TypeKind::Unit => {}
        }
    }

    fn check_type_path_leak(&mut self, owner: &str, path: &TypePath, generics: &BTreeSet<String>) {
        let name = &path.segments[0];
        if !generics.contains(name) {
            let leaked = if path.segments.len() == 1 {
                self.symbols
                    .types
                    .get(name)
                    .is_some_and(|symbol| symbol.visibility == Visibility::Private)
            } else {
                self.symbols
                    .namespaces
                    .get(name)
                    .into_iter()
                    .chain(self.symbols.types.get(name))
                    .any(|symbol| symbol.visibility == Visibility::Private)
            };

            if leaked {
                self.error(
                    PRIVATE_LEAK,
                    format!("Private declaration '{name}' leaks through public API '{owner}'."),
                    path.span,
                    "Export the private declaration or keep the API private.",
                );
            }
        }

        for argument in &path.arguments {
            self.check_type_leak(owner, argument, generics);
        }
    }
}

fn symbol_kind(kind: ItemKind) -> SymbolKind {
    match kind {
        ItemKind::Function => SymbolKind::Function,
        ItemKind::Struct => SymbolKind::Struct,
        ItemKind::Enum => SymbolKind::Enum,
        ItemKind::Constant => SymbolKind::Constant,
        ItemKind::TypeAlias => SymbolKind::TypeAlias,
        ItemKind::ForeignModule => SymbolKind::ForeignModule,
        ItemKind::ScriptBlock => SymbolKind::ScriptBlock,
    }
}

fn generic_names(parameters: &[String]) -> BTreeSet<String> {
    parameters.iter().cloned().collect()
}
