use std::collections::{BTreeMap, BTreeSet, HashMap};

use crate::ast;
use crate::diagnostic::Diagnostic;
use crate::hir;
use crate::mode::Mode;
use crate::resolver::resolve_module;
use crate::span::Span;
use crate::type_surface::{ImportedTypeDef, ModuleTypeSurface};
use crate::types::{FunctionType, Type};

const TYPE_MISMATCH: &str = "NS4001";
const UNKNOWN_IDENTIFIER: &str = "NS4002";
const UNKNOWN_TYPE: &str = "NS4003";
const ARGUMENT_COUNT: &str = "NS4004";
const NOT_CALLABLE: &str = "NS4005";
const INVALID_OPERANDS: &str = "NS4006";
const MISSING_RETURN: &str = "NS4007";
const NOT_ASSIGNABLE: &str = "NS4009";
const NON_EXHAUSTIVE_MATCH: &str = "NS4010";
const UNHANDLED_ERROR: &str = "NS4011";
const UNDECLARED_ERROR: &str = "NS4012";
const INVALID_PROPAGATION: &str = "NS4013";
const INVALID_CONTROL_FLOW: &str = "NS4014";
const INVALID_MEMBER: &str = "NS4015";
const INVALID_STRUCT_FIELD: &str = "NS4016";
const INVALID_PATTERN: &str = "NS4017";
const INVALID_TYPE_ARGUMENT: &str = "NS4018";
const TASK_BOUNDARY: &str = "NS4019";
const MODE_REQUIREMENT: &str = "NS5001";
const MODE_WIDENING: &str = "NS5002";
const CAPABILITY_REQUIREMENT: &str = "NS5003";
const MEMORY_REQUIREMENT: &str = "NS5004";
const INVALID_CAPABILITY_ANNOTATION: &str = "NS5005";
const MANUAL_ALLOC_IN_GC_SCOPE: &str = "NS6001";
const MANAGED_ELEMENT_IN_MANUAL_ALLOC: &str = "NS6002";
const UNSUPPORTED_SCRIPT_TARGET: &str = "NS7002";
const INVALID_SCRIPT_SIGNATURE: &str = "NS7005";

const NATIVE_ORDER_EFFECTS: &str = "NS5007";

const EXPORT_AS_TARGET: &str = "NS7010";

const EXPORT_AS_BINDINGS: &str = "NS7011";

const EXPORT_AS_TYPE: &str = "NS7012";

const EXPORT_AS_SYMBOL: &str = "NS7013";

#[derive(Debug, Clone, Default)]
pub struct CheckOptions {
    pub mode: Mode,

    pub target: Option<String>,

    pub module_surfaces: BTreeMap<String, ModuleTypeSurface>,
}

impl CheckOptions {
    #[must_use]
    pub fn new(mode: Mode) -> Self {
        Self {
            mode,
            target: None,
            module_surfaces: BTreeMap::new(),
        }
    }

    #[must_use]
    pub fn with_target(mut self, target: impl Into<String>) -> Self {
        self.target = Some(target.into());
        self
    }

    #[must_use]
    pub fn with_surfaces(mode: Mode, module_surfaces: BTreeMap<String, ModuleTypeSurface>) -> Self {
        Self {
            mode,
            target: None,
            module_surfaces,
        }
    }
}

impl PartialEq for CheckOptions {
    fn eq(&self, other: &Self) -> bool {
        self.mode == other.mode
            && self.target == other.target
            && self.module_surfaces == other.module_surfaces
    }
}

impl Eq for CheckOptions {}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct TypeAnnotation {
    pub span: Span,
    pub ty: Type,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CheckResult {
    pub module: hir::Module,

    pub annotations: Vec<TypeAnnotation>,

    pub manual_memory_scopes: Vec<Span>,
}

pub fn check_source(source: &str, options: CheckOptions) -> Result<CheckResult, Vec<Diagnostic>> {
    let module = crate::parser::parse(source)?;
    check_module(&module, options)
}

pub fn check_module(
    source: &ast::Module,
    options: CheckOptions,
) -> Result<CheckResult, Vec<Diagnostic>> {
    let resolved = resolve_module(source)?;
    check_resolved(source, &resolved, options)
}

pub fn check_resolved(
    source: &ast::Module,
    resolved: &hir::Module,
    options: CheckOptions,
) -> Result<CheckResult, Vec<Diagnostic>> {
    let mut checker = Checker::new(source, resolved, options);
    checker.run();

    if checker.diagnostics.is_empty() {
        Ok(CheckResult {
            module: checker.resolved,
            annotations: checker.annotations,
            manual_memory_scopes: checker.manual_memory_scopes,
        })
    } else {
        Err(checker.diagnostics)
    }
}

#[derive(Debug, Clone)]
struct Local {
    ty: Type,
    mutable: bool,
    span: Span,
}

#[derive(Debug, Clone)]
struct ValueDef {
    ty: Type,
}

impl ValueDef {
    fn new(ty: Type) -> Self {
        Self { ty }
    }
}

#[derive(Debug, Clone)]
struct NamespaceDef {
    members: BTreeMap<String, ValueDef>,
    open: bool,
}

#[derive(Debug, Clone)]
enum TypeDef {
    Struct(StructDef),
    Enum(EnumDef),
    Alias(AliasDef),
    BuiltinNominal,
}

#[derive(Debug, Clone)]
struct StructDef {
    type_parameters: Vec<String>,
    fields: ast::StructFields,
}

#[derive(Debug, Clone)]
struct EnumDef {
    type_parameters: Vec<String>,
    variants: Vec<ast::EnumVariant>,
}

#[derive(Debug, Clone)]
struct AliasDef {
    type_parameters: Vec<String>,
    ty: ast::Type,
}

#[derive(Debug, Clone)]
struct ResolvedField {
    name: Option<String>,
    ty: Type,
}

#[derive(Debug, Clone)]
struct ResolvedStruct {
    type_parameters: Vec<String>,
    fields: Vec<ResolvedField>,
}

#[derive(Debug, Clone)]
struct ResolvedVariant {
    name: String,
    fields: Option<Vec<ResolvedField>>,
}

#[derive(Debug, Clone)]
struct ResolvedEnum {
    type_parameters: Vec<String>,
    variants: Vec<ResolvedVariant>,
}

#[derive(Debug, Clone)]
struct FunctionContext {
    name: String,
    return_type: Type,
    declared_errors: Vec<Type>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ErrorEffect {
    ty: Type,
    span: Span,
}

impl ErrorEffect {
    fn new(ty: Type, span: Span) -> Self {
        Self { ty, span }
    }
}

#[derive(Debug, Clone)]
struct ExprInfo {
    ty: Type,
    errors: Vec<ErrorEffect>,
}

impl ExprInfo {
    fn value(ty: Type) -> Self {
        Self {
            ty,
            errors: Vec::new(),
        }
    }

    fn with_errors(mut self, errors: Vec<ErrorEffect>) -> Self {
        append_effects(&mut self.errors, errors);
        self
    }
}

#[derive(Debug, Clone)]
struct LValueInfo {
    ty: Type,
    mutable: bool,
    errors: Vec<ErrorEffect>,
}

#[derive(Debug, Clone)]
struct StmtInfo {
    expression: Option<ExprInfo>,
    diverges: bool,
}

#[derive(Debug, Clone)]
struct BlockInfo {
    tail: Option<ExprInfo>,
    diverges: bool,
}

#[derive(Debug, Clone, Default)]
struct Coverage {
    some: bool,
    none: bool,
    ok: bool,
    err: bool,
    boolean: [bool; 2],
    variants: BTreeSet<String>,
}

enum MatchShape {
    Option,
    Result,
    Enum(ResolvedEnum),
    Bool,
    Tuple,
    Other,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum ConstructorKey {
    Bool(bool),
    Some,
    None,
    Ok,
    Err,
    Variant(String),
    Tuple,
    Unit,
}

struct Constructor {
    key: ConstructorKey,
    fields: Vec<Type>,
}

#[derive(Clone, Copy)]
enum RowPattern<'a> {
    Wildcard,
    Pattern(&'a ast::Pattern),
}

struct Checker<'a> {
    source: &'a ast::Module,
    resolved: hir::Module,
    options: CheckOptions,
    diagnostics: Vec<Diagnostic>,
    annotations: Vec<TypeAnnotation>,
    manual_memory_scopes: Vec<Span>,

    manual_alloc_spans: Vec<Span>,

    native_order_suppress: usize,

    deferred_alloc_checks: BTreeMap<String, Vec<(Span, Type)>>,

    pending_generic_call_checks: Vec<(Span, String, String, BTreeMap<String, Type>)>,
    mode: Mode,
    function_modes: HashMap<Span, Mode>,
    type_defs: BTreeMap<String, TypeDef>,
    imported_types: BTreeSet<String>,
    values: BTreeMap<String, ValueDef>,
    namespaces: BTreeMap<String, NamespaceDef>,
    script_modes: BTreeMap<String, Mode>,
    scopes: Vec<BTreeMap<String, Local>>,
    current_function: Option<FunctionContext>,
    current_type_parameters: Vec<String>,
    loop_depth: usize,
    alias_stack: Vec<String>,
}

impl<'a> Checker<'a> {
    fn new(source: &'a ast::Module, resolved: &hir::Module, options: CheckOptions) -> Self {
        let mode = options.mode;
        Self {
            source,
            resolved: resolved.clone(),
            options,
            diagnostics: Vec::new(),
            annotations: Vec::new(),
            manual_memory_scopes: Vec::new(),
            manual_alloc_spans: Vec::new(),
            native_order_suppress: 0,
            deferred_alloc_checks: BTreeMap::new(),
            pending_generic_call_checks: Vec::new(),
            mode,
            function_modes: HashMap::new(),
            type_defs: BTreeMap::new(),
            imported_types: BTreeSet::new(),
            values: BTreeMap::new(),
            namespaces: BTreeMap::new(),
            script_modes: BTreeMap::new(),
            scopes: Vec::new(),
            current_function: None,
            current_type_parameters: Vec::new(),
            loop_depth: 0,
            alias_stack: Vec::new(),
        }
    }

    fn run(&mut self) {
        self.register_type_names();

        self.register_imports();
        self.register_values();
        self.check_mode_annotations();
        self.fill_type_definitions();
        self.check_constants();
        self.check_functions();
        self.check_deferred_generic_allocs();
    }

    fn error(
        &mut self,
        code: &str,
        message: impl Into<String>,
        span: Span,
        hint: impl Into<String>,
    ) {
        self.diagnostics.push(
            Diagnostic::error(code, message)
                .with_span(span)
                .with_hint(hint),
        );
    }

    fn annotate(&mut self, span: Span, ty: Type) {
        self.annotations.push(TypeAnnotation { span, ty });
    }

    fn register_type_names(&mut self) {
        let items = self.resolved.items.clone();
        for item in &items {
            match &item.declaration {
                hir::Declaration::Struct(declaration) => {
                    self.type_defs.insert(
                        declaration.name.clone(),
                        TypeDef::Struct(StructDef {
                            type_parameters: declaration.type_parameters.clone(),
                            fields: declaration.fields.clone(),
                        }),
                    );
                }
                hir::Declaration::Enum(declaration) => {
                    self.type_defs.insert(
                        declaration.name.clone(),
                        TypeDef::Enum(EnumDef {
                            type_parameters: declaration.type_parameters.clone(),
                            variants: declaration.variants.clone(),
                        }),
                    );
                }
                hir::Declaration::TypeAlias(declaration) => {
                    self.type_defs.insert(
                        declaration.name.clone(),
                        TypeDef::Alias(AliasDef {
                            type_parameters: declaration.type_parameters.clone(),
                            ty: declaration.ty.clone(),
                        }),
                    );
                }
                _ => {}
            }
        }

        for (name, def) in [
            ("ParseIntError", TypeDef::BuiltinNominal),
            ("OverflowError", TypeDef::BuiltinNominal),
            ("TaskError", TypeDef::BuiltinNominal),
            ("ChannelError", TypeDef::BuiltinNominal),
            ("SliceError", TypeDef::BuiltinNominal),
        ] {
            self.type_defs.entry(name.to_owned()).or_insert(def);
        }
    }

    fn register_values(&mut self) {
        let items = self.resolved.items.clone();
        for (index, item) in items.iter().enumerate() {
            match &item.declaration {
                hir::Declaration::Function(declaration) => {
                    let signature = self.function_signature(declaration);
                    self.values.insert(
                        declaration.name.clone(),
                        ValueDef::new(Type::Function(signature.clone())),
                    );
                    self.resolved.items[index].ty = Some(Type::Function(signature));
                }
                hir::Declaration::Struct(declaration) => {
                    let ty = Type::Named(
                        declaration.name.clone(),
                        declaration
                            .type_parameters
                            .iter()
                            .map(|parameter| Type::Generic(parameter.clone()))
                            .collect(),
                    );
                    if let ast::StructFields::Tuple(fields) = &declaration.fields {
                        let generics = generic_map(&declaration.type_parameters);
                        let parameters = fields
                            .iter()
                            .map(|field| self.resolve_type(field, &generics))
                            .collect::<Vec<_>>();
                        if !parameters.is_empty() {
                            let signature = FunctionType {
                                type_parameters: declaration.type_parameters.clone(),
                                parameters,
                                return_type: Box::new(ty.clone()),
                                errors: Vec::new(),
                            };
                            self.values.insert(
                                declaration.name.clone(),
                                ValueDef::new(Type::Function(signature)),
                            );
                        }
                    }
                    self.resolved.items[index].ty = Some(ty);
                }
                hir::Declaration::Enum(declaration) => {
                    let ty = Type::Named(
                        declaration.name.clone(),
                        declaration
                            .type_parameters
                            .iter()
                            .map(|parameter| Type::Generic(parameter.clone()))
                            .collect(),
                    );
                    self.resolved.items[index].ty = Some(ty);
                }
                hir::Declaration::Constant(declaration) => {
                    let generics = BTreeMap::new();
                    let ty = self.resolve_type(&declaration.ty, &generics);
                    self.values
                        .insert(declaration.name.clone(), ValueDef::new(ty.clone()));
                    self.resolved.items[index].ty = Some(ty);
                }
                hir::Declaration::TypeAlias(declaration) => {
                    let generics = generic_map(&declaration.type_parameters);
                    let ty = self.resolve_type(&declaration.ty, &generics);
                    self.resolved.items[index].ty = Some(ty);
                }
                hir::Declaration::ForeignModule(declaration) => {
                    let mut members = BTreeMap::new();
                    for function in &declaration.functions {
                        let signature = self.function_signature_parts(
                            function.name.clone(),
                            &[],
                            &function.parameters,
                            function.return_type.as_ref(),
                            &function.error_set,
                            function.span,
                        );
                        members.insert(
                            function.name.clone(),
                            ValueDef::new(Type::Function(signature)),
                        );
                    }
                    self.namespaces.insert(
                        declaration.name.clone(),
                        NamespaceDef {
                            members,
                            open: false,
                        },
                    );
                }
                hir::Declaration::ScriptBlock(declaration) => {
                    let mut members = BTreeMap::new();
                    for export in &declaration.exports {
                        let signature = self.function_signature_parts(
                            export.name.clone(),
                            &[],
                            &export.parameters,
                            export.return_type.as_ref(),
                            &export.error_set,
                            export.span,
                        );
                        members.insert(
                            export.name.clone(),
                            ValueDef::new(Type::Function(signature)),
                        );
                    }
                    self.namespaces.insert(
                        declaration.name.clone(),
                        NamespaceDef {
                            members,
                            open: false,
                        },
                    );
                }
            }
        }
    }

    fn register_imports(&mut self) {
        let mut local_to_import: BTreeMap<String, (String, String)> = BTreeMap::new();
        for import in &self.resolved.imports {
            match &import.kind {
                hir::ImportKind::Named { bindings } => {
                    for binding in bindings {
                        local_to_import.insert(
                            binding.local.clone(),
                            (import.module.clone(), binding.imported.clone()),
                        );
                    }
                }
                hir::ImportKind::Default { local } => {
                    local_to_import
                        .insert(local.clone(), (import.module.clone(), "default".to_owned()));
                }
                hir::ImportKind::DefaultNamed { default, bindings } => {
                    local_to_import.insert(
                        default.clone(),
                        (import.module.clone(), "default".to_owned()),
                    );
                    for binding in bindings {
                        local_to_import.insert(
                            binding.local.clone(),
                            (import.module.clone(), binding.imported.clone()),
                        );
                    }
                }
                hir::ImportKind::SideEffect | hir::ImportKind::Namespace { .. } => {}
            }
        }

        for (name, symbol) in &self.resolved.symbols.values {
            if self.values.contains_key(name) {
                continue;
            }
            if matches!(
                symbol.kind,
                hir::SymbolKind::ImportedValue | hir::SymbolKind::ImportedType
            ) {
                let ty = local_to_import
                    .get(name)
                    .and_then(|(module_key, imported_name)| {
                        self.options
                            .module_surfaces
                            .get(module_key)
                            .and_then(|surface| surface.get_value(imported_name))
                            .cloned()
                    })
                    .unwrap_or(Type::Unknown);

                self.values.insert(name.clone(), ValueDef::new(ty));
            }
        }

        let mut imported_constructors = Vec::new();
        for name in self.resolved.symbols.types.keys() {
            if self.type_defs.contains_key(name) {
                continue;
            }

            let def_from_surface =
                local_to_import
                    .get(name)
                    .and_then(|(module_key, imported_name)| {
                        self.options
                            .module_surfaces
                            .get(module_key)
                            .and_then(|surface| surface.get_type_def(imported_name))
                            .cloned()
                    });

            if let Some(imported_def) = def_from_surface {
                match imported_def {
                    ImportedTypeDef::Struct {
                        name: struct_name,
                        type_parameters,
                        fields,
                    } => {
                        if let ast::StructFields::Tuple(tuple_fields) = &fields
                            && !tuple_fields.is_empty()
                        {
                            imported_constructors.push((
                                name.clone(),
                                struct_name.clone(),
                                type_parameters.clone(),
                                tuple_fields.clone(),
                            ));
                        }
                        self.type_defs.insert(
                            name.clone(),
                            TypeDef::Struct(StructDef {
                                type_parameters,
                                fields,
                            }),
                        );

                        let ty = Type::Named(struct_name, Vec::new());
                        self.values
                            .entry(name.clone())
                            .or_insert_with(|| ValueDef::new(ty));
                    }
                    ImportedTypeDef::Enum {
                        name: enum_name,
                        type_parameters,
                        variants,
                    } => {
                        self.type_defs.insert(
                            name.clone(),
                            TypeDef::Enum(EnumDef {
                                type_parameters,
                                variants,
                            }),
                        );
                        let ty = Type::Named(enum_name, Vec::new());
                        self.values
                            .entry(name.clone())
                            .or_insert_with(|| ValueDef::new(ty));
                    }
                }
            } else {
                self.imported_types.insert(name.clone());
            }
        }

        for (local_name, struct_name, type_parameters, fields) in imported_constructors {
            let generics = generic_map(&type_parameters);
            let parameters = fields
                .iter()
                .map(|field| self.resolve_type(field, &generics))
                .collect::<Vec<_>>();
            let return_type = Type::Named(
                struct_name,
                type_parameters
                    .iter()
                    .map(|parameter| Type::Generic(parameter.clone()))
                    .collect(),
            );
            let signature = FunctionType {
                type_parameters,
                parameters,
                return_type: Box::new(return_type),
                errors: Vec::new(),
            };
            self.values
                .insert(local_name, ValueDef::new(Type::Function(signature)));
        }

        for (name, import) in &self.resolved.symbols.namespaces {
            if let hir::SymbolOrigin::Imported {
                module,
                name: imported,
            } = &import.origin
                && let Some(surface) = self.options.module_surfaces.get(module)
            {
                if imported == "*" {
                    let members: BTreeMap<String, ValueDef> = surface
                        .values
                        .iter()
                        .map(|(export_name, ty)| (export_name.clone(), ValueDef::new(ty.clone())))
                        .collect();
                    self.namespaces
                        .entry(name.clone())
                        .or_insert_with(|| NamespaceDef {
                            members,
                            open: false,
                        });
                    continue;
                }
                if let Some(members) = surface.namespaces.get(imported) {
                    let members = members
                        .iter()
                        .map(|(member, ty)| (member.clone(), ValueDef::new(ty.clone())))
                        .collect();
                    self.namespaces
                        .entry(name.clone())
                        .or_insert_with(|| NamespaceDef {
                            members,
                            open: false,
                        });
                    continue;
                }

                self.namespaces
                    .entry(name.clone())
                    .or_insert_with(|| NamespaceDef {
                        members: BTreeMap::new(),
                        open: true,
                    });
                continue;
            }
            self.namespaces
                .entry(name.clone())
                .or_insert_with(|| NamespaceDef {
                    members: BTreeMap::new(),
                    open: true,
                });
        }
    }

    fn fill_type_definitions(&mut self) {
        let items = self.resolved.items.clone();
        for (index, item) in items.iter().enumerate() {
            if let hir::Declaration::TypeAlias(declaration) = &item.declaration {
                let generics = generic_map(&declaration.type_parameters);
                let ty = self.resolve_type(&declaration.ty, &generics);
                self.resolved.items[index].ty = Some(ty);
            }
        }

        let names = self.type_defs.keys().cloned().collect::<Vec<_>>();
        for name in names {
            match self.type_defs.get(&name) {
                Some(TypeDef::Struct(_)) => {
                    let _ = self.resolve_struct(&name);
                }
                Some(TypeDef::Enum(_)) => {
                    let _ = self.resolve_enum(&name);
                }
                Some(TypeDef::Alias(_) | TypeDef::BuiltinNominal) | None => {}
            }
        }
    }

    fn check_constants(&mut self) {
        let items = self.resolved.items.clone();
        for item in &items {
            let hir::Declaration::Constant(declaration) = &item.declaration else {
                continue;
            };

            let previous_mode = self.mode;
            self.mode = self
                .function_modes
                .get(&declaration.span)
                .copied()
                .unwrap_or(self.options.mode);
            self.push_scope();
            let declared = self.resolve_type(&declaration.ty, &BTreeMap::new());
            let info = self.check_expr(&declaration.value, Some(&declared));
            self.report_effects(&info.errors);
            self.pop_scope();
            self.mode = previous_mode;
        }
    }

    fn check_functions(&mut self) {
        let items = self.resolved.items.clone();
        for item in &items {
            let hir::Declaration::Function(declaration) = &item.declaration else {
                continue;
            };

            let previous_mode = self.mode;
            self.mode = self
                .function_modes
                .get(&declaration.span)
                .copied()
                .unwrap_or(self.options.mode);

            let signature = match &item.ty {
                Some(Type::Function(signature)) => signature.clone(),
                _ => self.function_signature(declaration),
            };

            self.push_scope();
            for (parameter, parameter_type) in
                declaration.parameters.iter().zip(&signature.parameters)
            {
                self.insert_local(
                    parameter.name.clone(),
                    Local {
                        ty: parameter_type.clone(),
                        mutable: false,
                        span: parameter.span,
                    },
                );
            }

            self.current_type_parameters = declaration.type_parameters.clone();

            self.current_function = Some(FunctionContext {
                name: declaration.name.clone(),
                return_type: (*signature.return_type).clone(),
                declared_errors: signature.errors.clone(),
            });

            let body = self.check_block(&declaration.body, false, None);
            let return_type = (*signature.return_type).clone();
            if !return_type.accepts(&Type::Unit) && !body.diverges && !return_type.is_opaque() {
                self.error(
                    MISSING_RETURN,
                    format!(
                        "Function '{}' can reach the end of its body without returning '{}'.",
                        declaration.name, return_type
                    ),
                    declaration.span,
                    "Return a value on every path or change the return type to '()'.",
                );
            }

            self.current_function = None;
            self.current_type_parameters.clear();
            self.pop_scope();
            self.mode = previous_mode;
        }
    }

    fn check_deferred_generic_allocs(&mut self) {
        type PendingCall = (Span, String, BTreeMap<String, Type>);
        let mut by_enclosing: BTreeMap<String, Vec<PendingCall>> = BTreeMap::new();
        for (span, callee_name, enclosing, subs) in
            std::mem::take(&mut self.pending_generic_call_checks)
        {
            by_enclosing
                .entry(enclosing)
                .or_default()
                .push((span, callee_name, subs));
        }

        let mut worklist: Vec<(Span, String, BTreeMap<String, Type>)> = by_enclosing
            .values()
            .flat_map(|v| v.iter().cloned())
            .collect();
        let mut seen: std::collections::HashSet<(Span, String, BTreeMap<String, Type>)> =
            std::collections::HashSet::new();

        while let Some((call_span, callee_name, substitutions)) = worklist.pop() {
            if !seen.insert((call_span, callee_name.clone(), substitutions.clone())) {
                continue;
            }
            let all_concrete =
                !substitutions.is_empty() && substitutions.values().all(|t| !contains_generic(t));

            if all_concrete
                && let Some(checks) = self.deferred_alloc_checks.get(&callee_name).cloned()
            {
                for (_alloc_span, element_type) in checks {
                    let concrete_element = element_type.substitute(&substitutions);
                    if matches!(concrete_element, Type::Error | Type::Unknown) {
                        continue;
                    }
                    if contains_generic(&concrete_element) {
                        continue;
                    }
                    if self.holds_managed_refs(&concrete_element) {
                        self.error(
                                MANAGED_ELEMENT_IN_MANUAL_ALLOC,
                                format!(
                                    "Manual allocation cannot use element type '{concrete_element}': it holds \
                                     GC-managed references, and 'Ptr<T>' is never a GC root (spec 8.4), \
                                     so a collection could reclaim data this pointer still points at."
                                ),
                                call_span,
                                "Allocate pointer-free types here (scalars, Ptr<U>, or a struct/enum of \
                                 those); str, bytes, [T], Map, Channel, Task, and composites containing \
                                 them belong to GC-managed memory.",
                            );
                    }
                }
            }

            if all_concrete && let Some(inner_calls) = by_enclosing.get(&callee_name).cloned() {
                for (inner_span, inner_callee, inner_subs) in inner_calls {
                    let mut composed = inner_subs.clone();
                    for v in composed.values_mut() {
                        *v = v.substitute(&substitutions);
                    }

                    for (k, v) in &substitutions {
                        composed.entry(k.clone()).or_insert(v.clone());
                    }
                    worklist.push((inner_span, inner_callee, composed));
                }
            }
        }
    }

    fn function_signature(&mut self, declaration: &ast::FunctionDecl) -> FunctionType {
        self.function_signature_parts(
            declaration.name.clone(),
            &declaration.type_parameters,
            &declaration.parameters,
            declaration.return_type.as_ref(),
            &declaration.error_set,
            declaration.span,
        )
    }

    fn function_signature_parts(
        &mut self,
        name: String,
        type_parameters: &[String],
        parameters: &[ast::Parameter],
        return_type: Option<&ast::Type>,
        error_set: &[ast::TypePath],
        _span: Span,
    ) -> FunctionType {
        let generics = generic_map(type_parameters);
        let parameters = parameters
            .iter()
            .map(|parameter| self.resolve_type(&parameter.ty, &generics))
            .collect::<Vec<_>>();
        let return_type = return_type
            .map(|ty| self.resolve_type(ty, &generics))
            .unwrap_or(Type::Unit);
        let mut errors = Vec::new();
        for error in error_set {
            let resolved = self.resolve_type_path(error, &generics);
            if !matches!(resolved, Type::Named(_, _) | Type::Unknown | Type::Error) {
                self.error(
                    UNDECLARED_ERROR,
                    format!("The error set for '{name}' contains non-nominal type '{resolved}'."),
                    error.span,
                    "Declare a struct or enum error type and use its name after '!'.",
                );
            }
            append_unique(&mut errors, vec![resolved]);
        }

        FunctionType {
            type_parameters: type_parameters.to_vec(),
            parameters,
            return_type: Box::new(return_type),
            errors,
        }
    }

    fn push_scope(&mut self) {
        self.scopes.push(BTreeMap::new());
    }

    fn pop_scope(&mut self) {
        self.scopes.pop();
    }

    fn insert_local(&mut self, name: String, local: Local) {
        if name.is_empty() {
            return;
        }

        if let Some(scope) = self.scopes.last_mut()
            && let Some(existing) = scope.get(&name)
        {
            let span = existing.span;
            self.error(
                UNKNOWN_IDENTIFIER,
                format!("Identifier '{name}' is already declared in this scope."),
                local.span,
                format!(
                    "The previous declaration is at bytes {}..{}.",
                    span.start, span.end
                ),
            );
            return;
        }

        if let Some(scope) = self.scopes.last_mut() {
            scope.insert(name, local);
        }
    }

    fn lookup_local(&self, name: &str) -> Option<&Local> {
        self.scopes.iter().rev().find_map(|scope| scope.get(name))
    }

    fn check_mode_annotations(&mut self) {
        let module_mode = self.options.mode;
        self.mode = module_mode;
        let attributes = self.source.attributes.clone();
        self.check_capability_attributes(&attributes, module_mode);

        let items = self.source.items.clone();

        self.check_export_symbol_uniqueness(&items);
        self.check_item_modes(&items, module_mode);
        self.mode = module_mode;
    }

    fn check_item_modes(&mut self, items: &[ast::Item], current_mode: Mode) {
        for item in items {
            if !matches!(item.kind, ast::ItemKind::Function(_))
                && item
                    .attributes
                    .iter()
                    .any(|attribute| attribute.export_as.is_some())
            {
                let span = item
                    .attributes
                    .iter()
                    .find(|attribute| attribute.export_as.is_some())
                    .map(|attribute| attribute.span)
                    .unwrap_or(item.span);
                self.error(
                    EXPORT_AS_TARGET,
                    "@export_as is only allowed on a generic function declaration.",
                    span,
                    "Move the attribute onto a `pub fn`/`export fn` with type parameters.",
                );
            }
            match &item.kind {
                ast::ItemKind::ScopedItems(scoped) => {
                    let scoped_mode =
                        self.check_scope_annotation(&scoped.annotation, scoped.span, current_mode);
                    self.check_item_modes(&scoped.items, scoped_mode);
                }
                ast::ItemKind::Function(function) => {
                    self.function_modes.insert(function.span, current_mode);
                    self.check_capability_attributes(&item.attributes, current_mode);
                    self.check_export_as(item, function);
                }
                ast::ItemKind::Constant(constant) => {
                    self.function_modes.insert(constant.span, current_mode);
                    self.check_capability_attributes(&item.attributes, current_mode);
                }
                ast::ItemKind::Struct(_) | ast::ItemKind::Enum(_) | ast::ItemKind::TypeAlias(_) => {
                    self.check_capability_attributes(&item.attributes, current_mode);
                }
                ast::ItemKind::ForeignModule(foreign) => {
                    if !current_mode.allows(Mode::System) {
                        self.error(
                            MODE_REQUIREMENT,
                            format!(
                                "Foreign module '{}' requires system mode, but the active mode is '{}'.",
                                foreign.name,
                                current_mode.as_str()
                            ),
                            foreign.span,
                            "Move the foreign declaration behind a system-mode boundary.",
                        );
                    }
                    self.check_capability_attributes(&item.attributes, current_mode);
                }
                ast::ItemKind::Import(_) | ast::ItemKind::Export(_) => {
                    self.check_capability_attributes(&item.attributes, current_mode);
                }
                ast::ItemKind::ScriptBlock(script) => {
                    self.check_capability_attributes(&item.attributes, current_mode);

                    if let Some(target) = self.options.target.as_deref() {
                        let supported = match target {
                            "js" | "ts" => matches!(script.language.as_str(), "js" | "ts"),
                            "native" | "c" | "llvm" => {
                                matches!(script.language.as_str(), "c" | "cpp")
                            }
                            _ => false,
                        };
                        if !supported {
                            self.error(
                                UNSUPPORTED_SCRIPT_TARGET,
                                format!(
                                    "Script language '{}' is not supported by the active target '{target}'.",
                                    script.language
                                ),
                                script.span,
                                format!("Target '{target}' does not support '{}' script blocks.", script.language),
                            );
                        }
                    }

                    if let Some(mode) = Mode::parse(&script.mode) {
                        self.script_modes.insert(script.name.clone(), mode);
                    }

                    for export in &script.exports {
                        for param in &export.parameters {
                            let ty = self.resolve_type(&param.ty, &BTreeMap::new());
                            if !self.is_boundary_safe(&ty) {
                                self.error(
                                    INVALID_SCRIPT_SIGNATURE,
                                    format!("Type '{ty}' is not boundary-safe for script exports."),
                                    param.span,
                                    "Use boundary-safe primitive types, bool, str, bytes, Option, slices, or structs.",
                                );
                            }
                        }
                        if let Some(ret) = &export.return_type {
                            let ty = self.resolve_type(ret, &BTreeMap::new());
                            if !self.is_boundary_safe(&ty) {
                                self.error(
                                    INVALID_SCRIPT_SIGNATURE,
                                    format!("Type '{ty}' is not boundary-safe for script exports."),
                                    ret.span,
                                    "Use boundary-safe primitive types, bool, str, bytes, Option, slices, or structs.",
                                );
                            }
                        }
                    }
                }
            }
        }
    }

    fn check_export_symbol_uniqueness(&mut self, items: &[ast::Item]) {
        let mut symbols: BTreeMap<String, Span> = BTreeMap::new();
        self.check_export_symbols_in(items, &mut symbols);
    }

    fn check_export_symbols_in(
        &mut self,
        items: &[ast::Item],
        symbols: &mut BTreeMap<String, Span>,
    ) {
        for item in items {
            match &item.kind {
                ast::ItemKind::Function(function) => {
                    let exported = item.exported || item.visibility == ast::Visibility::Public;
                    if function.type_parameters.is_empty() || !exported {
                        continue;
                    }
                    for attribute in &item.attributes {
                        let Some(spec) = &attribute.export_as else {
                            continue;
                        };
                        if !is_valid_c_identifier(&spec.symbol) {
                            continue;
                        }
                        if symbols.contains_key(&spec.symbol) {
                            self.error(
                                EXPORT_AS_SYMBOL,
                                format!(
                                    "Exported symbol '{}' is already used by another `@export_as` in this module.",
                                    spec.symbol
                                ),
                                spec.span,
                                "Each exported symbol must be unique within the artifact.",
                            );
                        } else {
                            symbols.insert(spec.symbol.clone(), spec.span);
                        }
                    }
                }
                ast::ItemKind::ScopedItems(scoped) => {
                    self.check_export_symbols_in(&scoped.items, symbols);
                }
                _ => {}
            }
        }
    }

    fn check_export_as(&mut self, item: &ast::Item, function: &ast::FunctionDecl) {
        let specs: Vec<ast::ExportAsAttribute> = item
            .attributes
            .iter()
            .filter_map(|attribute| attribute.export_as.clone())
            .collect();
        if specs.is_empty() {
            return;
        }
        let exported = item.exported || item.visibility == ast::Visibility::Public;
        let mut instantiations = BTreeSet::new();
        for spec in specs {
            let span = spec.span;
            if function.type_parameters.is_empty() {
                self.error(
                    EXPORT_AS_TARGET,
                    "@export_as is only allowed on a generic function.",
                    span,
                    "Remove the attribute or add type parameters to the function.",
                );
                continue;
            }
            if !exported {
                self.error(
                    EXPORT_AS_TARGET,
                    "@export_as is only allowed on an exported (`pub` or `export`) function.",
                    span,
                    "Mark the declaration `pub` or `export`, or remove the attribute.",
                );
                continue;
            }
            let mut bound: BTreeMap<&str, &ast::Type> = BTreeMap::new();
            let mut valid = true;
            for (parameter, ty) in &spec.bindings {
                if !function
                    .type_parameters
                    .iter()
                    .any(|candidate| candidate == parameter)
                {
                    self.error(
                        EXPORT_AS_BINDINGS,
                        format!("`@export_as` binds unknown type parameter '{parameter}'."),
                        span,
                        "Bind each type parameter of the function exactly once.",
                    );
                    valid = false;
                    continue;
                }
                if bound.insert(parameter.as_str(), ty).is_some() {
                    self.error(
                        EXPORT_AS_BINDINGS,
                        format!("`@export_as` binds type parameter '{parameter}' more than once."),
                        span,
                        "Bind each type parameter of the function exactly once.",
                    );
                    valid = false;
                }
            }
            for parameter in &function.type_parameters {
                if !bound.contains_key(parameter.as_str()) {
                    self.error(
                        EXPORT_AS_BINDINGS,
                        format!(
                            "`@export_as` is missing a binding for type parameter '{parameter}'."
                        ),
                        span,
                        "Bind each type parameter of the function exactly once.",
                    );
                    valid = false;
                }
            }
            if !valid {
                continue;
            }
            let mut resolved: BTreeMap<String, Type> = BTreeMap::new();
            for (parameter, ty) in &bound {
                let resolved_ty = self.resolve_type(ty, &BTreeMap::new());
                if matches!(resolved_ty, Type::Error | Type::Unknown) {
                    valid = false;
                    continue;
                }
                if contains_generic(&resolved_ty) {
                    self.error(
                        EXPORT_AS_TYPE,
                        format!(
                            "`@export_as` bound type '{resolved_ty}' mentions a type parameter; it must be concrete."
                        ),
                        span,
                        "Bind a concrete, C-boundary-safe type.",
                    );
                    valid = false;
                    continue;
                }
                resolved.insert((*parameter).to_string(), resolved_ty);
            }
            if !valid {
                continue;
            }
            let mut boundary_ok = true;
            for ty in resolved.values() {
                if !self.is_boundary_safe(ty) {
                    self.error(
                        EXPORT_AS_TYPE,
                        format!("`@export_as` bound type '{ty}' is not C-boundary-safe."),
                        span,
                        "Use primitives, structs, enums, and composites of those (spec 11.4).",
                    );
                    boundary_ok = false;
                }
            }
            if !boundary_ok {
                continue;
            }
            if !is_valid_c_identifier(&spec.symbol) {
                self.error(
                    EXPORT_AS_SYMBOL,
                    format!("'{}' is not a valid C identifier.", spec.symbol),
                    span,
                    "Use letters, digits, and underscores, starting with a letter or underscore.",
                );
                continue;
            }

            let key: Vec<String> = function
                .type_parameters
                .iter()
                .map(|parameter| resolved[parameter].to_string())
                .collect();
            if !instantiations.insert(key) {
                self.error(
                    EXPORT_AS_SYMBOL,
                    "This instantiation is exported more than once.",
                    span,
                    "Remove the duplicate `@export_as`.",
                );
            }
        }
    }

    fn check_capability_attributes(&mut self, attributes: &[ast::Attribute], current_mode: Mode) {
        for attribute in attributes {
            match attribute.name.as_str() {
                "requires" => {
                    for argument in &attribute.arguments {
                        let Some((name, span)) = annotation_name(argument) else {
                            self.error(
                                INVALID_CAPABILITY_ANNOTATION,
                                "Attribute '@requires' expects a mode name.",
                                argument.span,
                                "Use core, app, web, or system.",
                            );
                            continue;
                        };
                        let Some(required) = Mode::parse(&name) else {
                            self.error(
                                INVALID_CAPABILITY_ANNOTATION,
                                format!("Unknown mode '{name}' in '@requires'."),
                                span,
                                "Use core, app, web, or system.",
                            );
                            continue;
                        };
                        if !current_mode.allows(required) {
                            self.error(
                                MODE_REQUIREMENT,
                                format!(
                                    "This item requires '{}' mode, but the active mode is '{}'.",
                                    required.as_str(),
                                    current_mode.as_str()
                                ),
                                attribute.span,
                                format!(
                                    "Mark the surrounding module or block @requires({}) or run with --mode {}.",
                                    required.as_str(),
                                    required.as_str()
                                ),
                            );
                        }
                    }
                }
                "tags" => {
                    for argument in &attribute.arguments {
                        let Some((tag, span)) = annotation_name(argument) else {
                            self.error(
                                INVALID_CAPABILITY_ANNOTATION,
                                "Attribute '@tags' expects capability names.",
                                argument.span,
                                "Use identifiers such as fs, net, dom, or ffi.",
                            );
                            continue;
                        };
                        let Some(required) = Mode::requirement_for_capability(&tag) else {
                            continue;
                        };
                        if !current_mode.allows(required) {
                            self.error(
                                CAPABILITY_REQUIREMENT,
                                format!(
                                    "Capability '{tag}' requires '{}' mode, but the active mode is '{}'.",
                                    required.as_str(),
                                    current_mode.as_str()
                                ),
                                span,
                                format!(
                                    "Run with --mode {} or remove the capability tag.",
                                    required.as_str()
                                ),
                            );
                        }
                    }
                }
                _ => {}
            }
        }
    }

    fn in_manual_memory_scope(&self, span: Span) -> bool {
        self.manual_memory_scopes
            .iter()
            .any(|scope| scope.contains(span.start))
    }

    fn check_manual_alloc_element(&mut self, callee: &ast::Expr, return_type: &Type, span: Span) {
        let target_span = match &callee.kind {
            ast::ExprKind::TypeArguments { target, .. } => target.span,
            _ => callee.span,
        };
        if !self.manual_alloc_spans.contains(&target_span) {
            return;
        }
        let Type::Named(name, arguments) = return_type else {
            return;
        };
        if name != "Ptr" {
            return;
        }
        let Some(element) = arguments.first() else {
            return;
        };
        if matches!(element, Type::Error | Type::Unknown) {
            return;
        }
        if contains_generic(element) {
            if let Some(current) = &self.current_function {
                self.deferred_alloc_checks
                    .entry(current.name.clone())
                    .or_default()
                    .push((span, element.clone()));
            }
            return;
        }
        if self.holds_managed_refs(element) {
            self.error(
                MANAGED_ELEMENT_IN_MANUAL_ALLOC,
                format!(
                    "Manual allocation cannot use element type '{element}': it holds \
                     GC-managed references, and 'Ptr<T>' is never a GC root (spec 8.4), \
                     so a collection could reclaim data this pointer still points at."
                ),
                span,
                "Allocate pointer-free types here (scalars, Ptr<U>, or a struct/enum of \
                 those); str, bytes, [T], Map, Channel, Task, and composites containing \
                 them belong to GC-managed memory.",
            );
        }
    }

    fn holds_managed_refs(&mut self, ty: &Type) -> bool {
        match ty {
            Type::Str | Type::Bytes | Type::Array(_) | Type::Slice(_) | Type::Map(_, _) => true,

            Type::FixedArray(element, _) => self.holds_managed_refs(element),
            Type::Tuple(elements) => elements
                .iter()
                .any(|element| self.holds_managed_refs(element)),
            Type::Option(inner) => self.holds_managed_refs(inner),
            Type::Result(ok, error) => {
                self.holds_managed_refs(ok) || self.holds_managed_refs(error)
            }
            Type::Named(name, arguments) => match name.as_str() {
                "Ptr" | "Region" => false,

                "Channel" | "Task" => true,
                _ => {
                    if let Some(struct_def) = self.resolve_struct(name) {
                        let substitutions = substitutions(&struct_def.type_parameters, arguments);
                        struct_def.fields.iter().any(|field| {
                            self.holds_managed_refs(&field.ty.substitute(&substitutions))
                        })
                    } else if let Some(enum_def) = self.resolve_enum(name) {
                        let substitutions = substitutions(&enum_def.type_parameters, arguments);
                        enum_def.variants.iter().any(|variant| {
                            variant.fields.iter().flatten().any(|field| {
                                self.holds_managed_refs(&field.ty.substitute(&substitutions))
                            })
                        })
                    } else {
                        false
                    }
                }
            },

            _ => false,
        }
    }

    fn contains_task(&mut self, ty: &Type) -> bool {
        match ty {
            Type::FixedArray(element, _)
            | Type::Array(element)
            | Type::Slice(element)
            | Type::Option(element) => self.contains_task(element),
            Type::Tuple(elements) => elements.iter().any(|element| self.contains_task(element)),
            Type::Result(ok, error) => self.contains_task(ok) || self.contains_task(error),
            Type::Map(key, value) => self.contains_task(key) || self.contains_task(value),
            Type::Named(name, arguments) => match name.as_str() {
                "Task" => true,

                "Channel" => arguments
                    .iter()
                    .any(|argument| self.contains_task(argument)),
                "Ptr" | "Region" => false,
                _ => {
                    if let Some(struct_def) = self.resolve_struct(name) {
                        let substitutions = substitutions(&struct_def.type_parameters, arguments);
                        struct_def
                            .fields
                            .iter()
                            .any(|field| self.contains_task(&field.ty.substitute(&substitutions)))
                    } else if let Some(enum_def) = self.resolve_enum(name) {
                        let substitutions = substitutions(&enum_def.type_parameters, arguments);
                        enum_def.variants.iter().any(|variant| {
                            variant.fields.iter().flatten().any(|field| {
                                self.contains_task(&field.ty.substitute(&substitutions))
                            })
                        })
                    } else {
                        false
                    }
                }
            },
            _ => false,
        }
    }

    fn check_channel_element(&mut self, callee: &ast::Expr, return_type: &Type, span: Span) {
        let Type::Named(name, arguments) = return_type else {
            return;
        };
        if name != "Channel" || self.resolve_struct(name).is_some() {
            return;
        }
        if !channel_create_callee(callee) {
            return;
        }
        let Some(element) = arguments.first() else {
            return;
        };
        if self.contains_task(element) {
            self.error(
                TASK_BOUNDARY,
                "Channel<T> cannot carry Task handles or values containing them.".to_owned(),
                span,
                "Send the task result instead; a task handle has a single owner.",
            );
        }
    }

    fn check_scope_annotation(
        &mut self,
        annotation: &ast::ScopeAnnotation,
        scope_span: Span,
        current_mode: Mode,
    ) -> Mode {
        match annotation.kind {
            ast::ScopeKind::Mode => {
                let Some(scoped_mode) = Mode::parse(&annotation.value) else {
                    self.error(
                        INVALID_CAPABILITY_ANNOTATION,
                        format!("Unknown mode '{}' in a scope annotation.", annotation.value),
                        annotation.span,
                        "Use core, app, web, or system.",
                    );
                    return current_mode;
                };
                if !current_mode.allows(scoped_mode) {
                    self.error(
                        MODE_WIDENING,
                        format!(
                            "Mode widening from '{}' to '{}' is not allowed.",
                            current_mode.as_str(),
                            scoped_mode.as_str()
                        ),
                        annotation.span,
                        "A scoped block may narrow capabilities, not widen them.",
                    );
                    return current_mode;
                }
                scoped_mode
            }
            ast::ScopeKind::Memory => {
                if annotation.value == "manual" {
                    self.manual_memory_scopes.push(scope_span);
                }
                if annotation.value == "manual" && !current_mode.allows(Mode::System) {
                    self.error(
                        MEMORY_REQUIREMENT,
                        format!(
                            "Manual memory requires system mode, but the active mode is '{}'.",
                            current_mode.as_str()
                        ),
                        annotation.span,
                        "Use '@memory:manual' only inside a system-mode boundary.",
                    );
                }
                current_mode
            }
        }
    }

    fn is_boundary_safe(&mut self, ty: &Type) -> bool {
        match ty {
            Type::I8
            | Type::I16
            | Type::I32
            | Type::I64
            | Type::U8
            | Type::U16
            | Type::U32
            | Type::U64
            | Type::Usize
            | Type::Isize
            | Type::F32
            | Type::F64
            | Type::Bool
            | Type::Str
            | Type::Bytes
            | Type::Unit => true,
            Type::Option(inner) => self.is_boundary_safe(inner),
            Type::Array(inner) | Type::Slice(inner) | Type::FixedArray(inner, _) => {
                self.is_boundary_safe(inner)
            }
            Type::Named(name, args) => {
                if let Some(def) = self.resolve_struct(name) {
                    def.fields.iter().all(|f| self.is_boundary_safe(&f.ty))
                } else {
                    args.iter().all(|arg| self.is_boundary_safe(arg))
                }
            }
            Type::Result(ok, err) => self.is_boundary_safe(ok) && self.is_boundary_safe(err),
            Type::Generic(_)
            | Type::Function(_)
            | Type::Tuple(_)
            | Type::Map(_, _)
            | Type::Never => false,
            Type::Error | Type::Unknown => true,
        }
    }

    fn resolve_type(&mut self, ty: &ast::Type, generics: &BTreeMap<String, Type>) -> Type {
        match &ty.kind {
            ast::TypeKind::Path(path) => self.resolve_type_path(path, generics),
            ast::TypeKind::Array(element) => {
                Type::Array(Box::new(self.resolve_type(element, generics)))
            }
            ast::TypeKind::FixedArray(element, length) => {
                let length = length
                    .replace('_', "")
                    .parse::<usize>()
                    .unwrap_or_else(|_| {
                        self.error(
                            UNKNOWN_TYPE,
                            format!("Invalid fixed-array length '{length}'."),
                            ty.span,
                            "Use a non-negative decimal integer.",
                        );
                        0
                    });
                Type::FixedArray(Box::new(self.resolve_type(element, generics)), length)
            }
            ast::TypeKind::Map(key, value) => Type::Map(
                Box::new(self.resolve_type(key, generics)),
                Box::new(self.resolve_type(value, generics)),
            ),
            ast::TypeKind::Tuple(elements) => Type::Tuple(
                elements
                    .iter()
                    .map(|element| self.resolve_type(element, generics))
                    .collect(),
            ),
            ast::TypeKind::Function {
                parameters,
                return_type,
                error_set,
            } => Type::Function(FunctionType {
                type_parameters: Vec::new(),
                parameters: parameters
                    .iter()
                    .map(|parameter| self.resolve_type(parameter, generics))
                    .collect(),
                return_type: Box::new(
                    return_type
                        .as_ref()
                        .map(|ty| self.resolve_type(ty, generics))
                        .unwrap_or(Type::Unit),
                ),
                errors: error_set
                    .iter()
                    .map(|error| self.resolve_type_path(error, generics))
                    .collect(),
            }),
            ast::TypeKind::Unit => Type::Unit,
        }
    }

    fn resolve_type_path(
        &mut self,
        path: &ast::TypePath,
        generics: &BTreeMap<String, Type>,
    ) -> Type {
        let name = path.segments.join(".");
        let short_name = path
            .segments
            .last()
            .cloned()
            .unwrap_or_else(|| name.clone());
        let arguments = path
            .arguments
            .iter()
            .map(|argument| self.resolve_type(argument, generics))
            .collect::<Vec<_>>();

        if path.arguments.is_empty()
            && let Some(generic) = generics.get(&short_name)
        {
            return generic.clone();
        }

        if !path.arguments.is_empty() && generics.contains_key(&short_name) {
            self.error(
                INVALID_TYPE_ARGUMENT,
                format!("Generic parameter '{short_name}' cannot take type arguments."),
                path.span,
                "Remove the type argument list.",
            );
            return Type::Error;
        }

        match short_name.as_str() {
            "i8" => return primitive_type(&arguments, Type::I8, path.span, self),
            "i16" => return primitive_type(&arguments, Type::I16, path.span, self),
            "i32" => return primitive_type(&arguments, Type::I32, path.span, self),
            "i64" => return primitive_type(&arguments, Type::I64, path.span, self),
            "u8" => return primitive_type(&arguments, Type::U8, path.span, self),
            "u16" => return primitive_type(&arguments, Type::U16, path.span, self),
            "u32" => return primitive_type(&arguments, Type::U32, path.span, self),
            "u64" => return primitive_type(&arguments, Type::U64, path.span, self),
            "usize" => return primitive_type(&arguments, Type::Usize, path.span, self),
            "isize" => return primitive_type(&arguments, Type::Isize, path.span, self),
            "f32" => return primitive_type(&arguments, Type::F32, path.span, self),
            "f64" => return primitive_type(&arguments, Type::F64, path.span, self),
            "bool" => return primitive_type(&arguments, Type::Bool, path.span, self),
            "str" => return primitive_type(&arguments, Type::Str, path.span, self),
            "bytes" => return primitive_type(&arguments, Type::Bytes, path.span, self),
            "Never" => return primitive_type(&arguments, Type::Never, path.span, self),
            "()" => return Type::Unit,
            "Option" => {
                if arguments.len() != 1 {
                    self.error(
                        INVALID_TYPE_ARGUMENT,
                        "Option requires exactly one type argument.",
                        path.span,
                        "Use Option<T>.",
                    );
                    return Type::Error;
                }
                return Type::Option(Box::new(arguments[0].clone()));
            }
            "Result" => {
                if arguments.len() != 2 {
                    self.error(
                        INVALID_TYPE_ARGUMENT,
                        "Result requires exactly two type arguments.",
                        path.span,
                        "Use Result<T, E>.",
                    );
                    return Type::Error;
                }
                return Type::Result(
                    Box::new(arguments[0].clone()),
                    Box::new(arguments[1].clone()),
                );
            }

            "Region" => {
                if !arguments.is_empty() {
                    self.error(
                        INVALID_TYPE_ARGUMENT,
                        "'Region' takes no type arguments.",
                        path.span,
                        "Use 'Region' on its own.",
                    );
                    return Type::Error;
                }
                return Type::Named("Region".to_owned(), Vec::new());
            }
            "Task" | "Ptr" | "MutSlice" | "Channel" => {
                if arguments.len() != 1 {
                    self.error(
                        INVALID_TYPE_ARGUMENT,
                        format!("{short_name} requires exactly one type argument."),
                        path.span,
                        format!("Use {short_name}<T>."),
                    );
                    return Type::Error;
                }
                return Type::Named(short_name, arguments);
            }
            _ => {}
        }

        if let Some(def) = self.type_defs.get(&short_name).cloned() {
            return self.resolve_declared_type(&short_name, def, arguments, path.span);
        }

        if self.imported_types.contains(&short_name) {
            return Type::Unknown;
        }

        if path.segments.len() > 1 {
            return Type::Unknown;
        }

        self.error(
            UNKNOWN_TYPE,
            format!("Unknown type '{name}'."),
            path.span,
            "Declare the type, import it, or use a built-in type.",
        );
        Type::Error
    }

    fn resolve_declared_type(
        &mut self,
        name: &str,
        def: TypeDef,
        arguments: Vec<Type>,
        span: Span,
    ) -> Type {
        match def {
            TypeDef::Struct(struct_def) => {
                if !check_arity(&struct_def.type_parameters, &arguments, name, span, self) {
                    return Type::Error;
                }
                Type::Named(name.to_owned(), arguments)
            }
            TypeDef::Enum(enum_def) => {
                if !check_arity(&enum_def.type_parameters, &arguments, name, span, self) {
                    return Type::Error;
                }
                Type::Named(name.to_owned(), arguments)
            }
            TypeDef::Alias(alias) => {
                if !check_arity(&alias.type_parameters, &arguments, name, span, self) {
                    return Type::Error;
                }
                if self.alias_stack.iter().any(|active| active == name) {
                    self.error(
                        UNKNOWN_TYPE,
                        format!("Cyclic type alias involving '{name}'."),
                        span,
                        "Break the alias cycle by naming a struct or enum.",
                    );
                    return Type::Error;
                }

                let mut substitutions = BTreeMap::new();
                for (parameter, argument) in alias.type_parameters.iter().zip(arguments) {
                    substitutions.insert(parameter.clone(), argument);
                }

                self.alias_stack.push(name.to_owned());
                let resolved = self.resolve_type(&alias.ty, &substitutions);
                self.alias_stack.pop();
                resolved
            }
            TypeDef::BuiltinNominal => Type::Named(name.to_owned(), arguments),
        }
    }

    fn resolve_struct(&mut self, name: &str) -> Option<ResolvedStruct> {
        let TypeDef::Struct(def) = self.type_defs.get(name)?.clone() else {
            return None;
        };
        let generics = generic_map(&def.type_parameters);
        let fields = match &def.fields {
            ast::StructFields::Named(fields) => fields
                .iter()
                .map(|field| ResolvedField {
                    name: Some(field.name.clone()),
                    ty: self.resolve_type(&field.ty, &generics),
                })
                .collect(),
            ast::StructFields::Tuple(fields) => fields
                .iter()
                .enumerate()
                .map(|(index, field)| ResolvedField {
                    name: Some(index.to_string()),
                    ty: self.resolve_type(field, &generics),
                })
                .collect(),
        };

        Some(ResolvedStruct {
            type_parameters: def.type_parameters,
            fields,
        })
    }

    fn resolve_enum(&mut self, name: &str) -> Option<ResolvedEnum> {
        let TypeDef::Enum(def) = self.type_defs.get(name)?.clone() else {
            return None;
        };
        let generics = generic_map(&def.type_parameters);
        let variants = def
            .variants
            .iter()
            .map(|variant| {
                let fields = variant.fields.as_ref().map(|fields| {
                    fields
                        .iter()
                        .enumerate()
                        .map(|(index, field)| ResolvedField {
                            name: field.name.clone().or_else(|| Some(index.to_string())),
                            ty: self.resolve_type(&field.ty, &generics),
                        })
                        .collect::<Vec<_>>()
                });
                ResolvedVariant {
                    name: variant.name.clone(),
                    fields,
                }
            })
            .collect();

        Some(ResolvedEnum {
            type_parameters: def.type_parameters,
            variants,
        })
    }

    fn check_block(
        &mut self,
        block: &ast::Block,
        collect_tail: bool,
        expected: Option<&Type>,
    ) -> BlockInfo {
        self.push_scope();
        let mut diverges = false;
        let mut tail = None;

        for (index, statement) in block.statements.iter().enumerate() {
            let is_last = index + 1 == block.statements.len();
            let tail_expected = if collect_tail && is_last {
                expected
            } else {
                None
            };
            let info = self.check_stmt(statement, tail_expected);
            if info.diverges {
                diverges = true;
            }

            if let Some(expression) = info.expression {
                if collect_tail && is_last {
                    tail = Some(expression);
                } else {
                    self.report_effects(&expression.errors);
                }
            }
        }

        self.pop_scope();

        BlockInfo { tail, diverges }
    }

    fn check_stmt(&mut self, statement: &ast::Stmt, expected: Option<&Type>) -> StmtInfo {
        match &statement.kind {
            ast::StmtKind::Let {
                mutable,
                name,
                ty,
                value,
            } => {
                let declared = ty
                    .as_ref()
                    .map(|ty| self.resolve_type(ty, &BTreeMap::new()));
                let info = self.check_expr(value, declared.as_ref());
                self.report_effects(&info.errors);
                let local_ty = declared.unwrap_or(info.ty);
                self.insert_local(
                    name.clone(),
                    Local {
                        ty: local_ty,
                        mutable: *mutable,
                        span: statement.span,
                    },
                );
                StmtInfo {
                    expression: None,
                    diverges: false,
                }
            }
            ast::StmtKind::Return { value } => {
                let return_type = self
                    .current_function
                    .as_ref()
                    .map(|function| function.return_type.clone());
                let Some(return_type) = return_type else {
                    self.error(
                        INVALID_CONTROL_FLOW,
                        "'return' is only valid inside a function.",
                        statement.span,
                        "Move the return into a function body.",
                    );
                    return StmtInfo {
                        expression: None,
                        diverges: true,
                    };
                };

                let info = match value {
                    Some(value) => self.check_expr(value, Some(&return_type)),
                    None => {
                        let info = ExprInfo::value(Type::Unit);
                        if !return_type.accepts(&Type::Unit) {
                            self.type_mismatch(&return_type, &Type::Unit, statement.span);
                        }
                        info
                    }
                };
                self.report_effects(&info.errors);
                StmtInfo {
                    expression: None,
                    diverges: true,
                }
            }
            ast::StmtKind::Break => {
                if self.loop_depth == 0 {
                    self.error(
                        INVALID_CONTROL_FLOW,
                        "'break' is only valid inside a loop.",
                        statement.span,
                        "Move the break into a while or for body.",
                    );
                }
                StmtInfo {
                    expression: None,
                    diverges: true,
                }
            }
            ast::StmtKind::Continue => {
                if self.loop_depth == 0 {
                    self.error(
                        INVALID_CONTROL_FLOW,
                        "'continue' is only valid inside a loop.",
                        statement.span,
                        "Move the continue into a while or for body.",
                    );
                }
                StmtInfo {
                    expression: None,
                    diverges: true,
                }
            }
            ast::StmtKind::Defer(deferred) => {
                let info = self.check_stmt(deferred, None);
                if let Some(expression) = info.expression {
                    self.report_effects(&expression.errors);
                }
                StmtInfo {
                    expression: None,
                    diverges: false,
                }
            }
            ast::StmtKind::While { condition, body } => {
                let condition = self.check_expr(condition, Some(&Type::Bool));
                self.report_effects(&condition.errors);
                self.loop_depth += 1;
                let _ = self.check_block(body, false, None);
                self.loop_depth -= 1;
                StmtInfo {
                    expression: None,
                    diverges: false,
                }
            }
            ast::StmtKind::For {
                binding,
                iterable,
                body,
            } => {
                let iterable = self.check_expr(iterable, None);
                self.report_effects(&iterable.errors);
                let element = self.iterable_element(&iterable.ty, statement.span);
                self.push_scope();
                self.insert_local(
                    binding.clone(),
                    Local {
                        ty: element,
                        mutable: false,
                        span: statement.span,
                    },
                );
                self.loop_depth += 1;
                let _ = self.check_block(body, false, None);
                self.loop_depth -= 1;
                self.pop_scope();
                StmtInfo {
                    expression: None,
                    diverges: false,
                }
            }
            ast::StmtKind::Expr(expression) => {
                let info = self.check_expr(expression, expected);
                StmtInfo {
                    diverges: info.ty == Type::Never,
                    expression: Some(info),
                }
            }
            ast::StmtKind::Scoped(scoped) => {
                let previous_mode = self.mode;
                self.mode =
                    self.check_scope_annotation(&scoped.annotation, scoped.span, previous_mode);
                self.push_scope();
                for statement in &scoped.statements {
                    let info = self.check_stmt(statement, None);
                    if let Some(expression) = info.expression {
                        self.report_effects(&expression.errors);
                    }
                }
                self.pop_scope();
                self.mode = previous_mode;
                StmtInfo {
                    expression: None,
                    diverges: false,
                }
            }
        }
    }

    fn iterable_element(&mut self, ty: &Type, span: Span) -> Type {
        match ty {
            Type::Array(element) | Type::Slice(element) | Type::FixedArray(element, _) => {
                (**element).clone()
            }
            Type::Map(key, _) => (**key).clone(),
            Type::Str => Type::Str,
            Type::Bytes => Type::U8,
            Type::Unknown | Type::Error => Type::Unknown,
            _ => {
                self.error(
                    INVALID_OPERANDS,
                    format!("Type '{ty}' is not iterable."),
                    span,
                    "Iterate over an array, slice, map, str, or bytes.",
                );
                Type::Error
            }
        }
    }

    fn check_expr(&mut self, expression: &ast::Expr, expected: Option<&Type>) -> ExprInfo {
        let info = self.check_expr_inner(expression, expected);
        if let Some(expected) = expected
            && !expected.accepts(&info.ty)
        {
            self.type_mismatch(expected, &info.ty, expression.span);
        }
        self.annotate(expression.span, info.ty.clone());
        info
    }

    fn check_native_order_effects(&mut self, expression: &ast::Expr) -> bool {
        if self.mode != Mode::System || self.native_order_suppress > 0 {
            return false;
        }
        let (effects, span) = match &expression.kind {
            ast::ExprKind::Binary { left, right, .. } => {
                let count = usize::from(expr_has_call(left)) + usize::from(expr_has_call(right));
                (count, expression.span)
            }
            ast::ExprKind::Array(elements) | ast::ExprKind::Tuple(elements) => (
                elements
                    .iter()
                    .filter(|element| expr_has_call(element))
                    .count(),
                expression.span,
            ),
            ast::ExprKind::Map(entries) => (
                entries
                    .iter()
                    .filter(|entry| expr_has_call(&entry.key) || expr_has_call(&entry.value))
                    .count(),
                expression.span,
            ),
            ast::ExprKind::StructInit { fields, .. } => (
                fields
                    .iter()
                    .filter(|field| expr_has_call(&field.value))
                    .count(),
                expression.span,
            ),
            _ => return false,
        };
        if effects < 2 {
            return false;
        }
        self.error(
            NATIVE_ORDER_EFFECTS,
            "This expression runs more than one state-changing call in a position \
             whose evaluation order is target-native, so the order is not portable \
             (spec 3.10).",
            span,
            "Hoist each call into its own `let` binding first; the bindings run in \
             source order on every backend.",
        );
        true
    }

    fn check_expr_inner(&mut self, expression: &ast::Expr, expected: Option<&Type>) -> ExprInfo {
        let flagged = self.check_native_order_effects(expression);
        if flagged {
            self.native_order_suppress += 1;
        }
        let info = self.check_expr_inner_body(expression, expected);
        if flagged {
            self.native_order_suppress -= 1;
        }
        info
    }

    fn check_expr_inner_body(
        &mut self,
        expression: &ast::Expr,
        expected: Option<&Type>,
    ) -> ExprInfo {
        match &expression.kind {
            ast::ExprKind::Literal(literal) => {
                ExprInfo::value(self.literal_type(literal, expected, expression.span))
            }
            ast::ExprKind::Identifier(name) => self.check_identifier(name, expression.span),
            ast::ExprKind::Unary { operator, operand } => {
                self.check_unary(*operator, operand, expression.span, expected)
            }
            ast::ExprKind::Binary {
                operator,
                left,
                right,
            } => self.check_binary(*operator, left, right, expression.span, expected),
            ast::ExprKind::Assign { target, value } => {
                self.check_assignment(target, value, expression.span)
            }
            ast::ExprKind::Call { callee, arguments } => {
                self.check_call(callee, arguments, expression.span, expected)
            }
            ast::ExprKind::Index { target, index } => {
                self.check_index(target, index, expression.span)
            }
            ast::ExprKind::Member { target, member } => {
                self.check_member(target, member, expression.span, expected)
            }
            ast::ExprKind::TypeArguments { target, arguments } => {
                for argument in arguments {
                    let _ = self.resolve_type(argument, &BTreeMap::new());
                }
                self.check_expr(target, expected)
            }
            ast::ExprKind::Propagate(inner) => self.check_propagation(inner, expression.span),
            ast::ExprKind::Array(elements) => self.check_array(elements, expression.span, expected),
            ast::ExprKind::Map(entries) => self.check_map(entries, expression.span, expected),
            ast::ExprKind::Tuple(elements) => self.check_tuple(elements, expression.span, expected),
            ast::ExprKind::StructInit { path, fields } => {
                self.check_struct_init(path, fields, expression.span)
            }
            ast::ExprKind::Block(block) => self.check_block_expression(block, expected),
            ast::ExprKind::If {
                condition,
                then_branch,
                else_branch,
            } => self.check_if(
                condition,
                then_branch,
                else_branch.as_ref(),
                expression.span,
                expected,
            ),
            ast::ExprKind::Match { value, arms } => {
                self.check_match(value, arms, expression.span, expected)
            }
            ast::ExprKind::Spawn { body } => {
                let info = self.check_block(body, true, None);
                let value_type = info
                    .tail
                    .as_ref()
                    .map_or(Type::Unit, |tail| tail.ty.clone());
                if let Some(tail) = info.tail {
                    self.report_effects(&tail.errors);
                }

                if self.contains_task(&value_type) {
                    self.error(
                        TASK_BOUNDARY,
                        "a spawn result cannot carry a Task handle; the child must join the task and return the value.".to_owned(),
                        expression.span,
                        "Return the task's result value instead of the handle.",
                    );
                }

                let free = crate::free_names::spawn_free_names(body);
                let mut reported_capture = false;
                for name in &free {
                    if reported_capture {
                        break;
                    }
                    if let Some(local) = self.lookup_local(name) {
                        let ty = local.ty.clone();
                        if self.contains_task(&ty) {
                            self.error(
                                TASK_BOUNDARY,
                                format!(
                                    "spawn captures a Task handle via `{name}: {ty}`; task handles cannot cross a task boundary."
                                ),
                                expression.span,
                                "Join the task inside the child and capture the result instead.",
                            );
                            reported_capture = true;
                        }
                    }
                }
                ExprInfo::value(Type::Named("Task".to_owned(), vec![value_type]))
            }
            ast::ExprKind::Fail { error, arguments } => {
                self.check_fail(error, arguments, expression.span)
            }
            ast::ExprKind::Panic { message } | ast::ExprKind::Abort { message } => {
                let info = self.check_expr(message, Some(&Type::Str));
                ExprInfo {
                    ty: Type::Never,
                    errors: info.errors,
                }
            }
        }
    }

    fn check_identifier(&mut self, name: &str, span: Span) -> ExprInfo {
        if let Some(local) = self.lookup_local(name) {
            return ExprInfo::value(local.ty.clone());
        }
        if let Some(value) = self.values.get(name) {
            return ExprInfo::value(value.ty.clone());
        }
        if name == "None" {
            return ExprInfo::value(Type::Option(Box::new(Type::Unknown)));
        }
        if let Some(ty) = builtin_function(name) {
            return ExprInfo::value(ty);
        }
        if self.type_defs.contains_key(name) || self.imported_types.contains(name) {
            self.error(
                UNKNOWN_IDENTIFIER,
                format!("Type '{name}' cannot be used as a value."),
                span,
                "Use the type in a type position or construct a value with its initializer.",
            );
            return ExprInfo::value(Type::Error);
        }
        if self.namespaces.contains_key(name) {
            self.error(
                UNKNOWN_IDENTIFIER,
                format!("Namespace '{name}' cannot be used as a value."),
                span,
                "Access one of its members instead.",
            );
            return ExprInfo::value(Type::Error);
        }

        self.error(
            UNKNOWN_IDENTIFIER,
            format!("Unknown identifier '{name}'."),
            span,
            "Declare the name before using it.",
        );
        ExprInfo::value(Type::Error)
    }

    fn check_unary(
        &mut self,
        operator: ast::UnaryOp,
        operand: &ast::Expr,
        span: Span,
        expected: Option<&Type>,
    ) -> ExprInfo {
        match operator {
            ast::UnaryOp::Negate => {
                let info = self.check_expr(operand, expected.filter(|ty| ty.is_numeric()));
                if info.ty == Type::Never {
                    return info;
                }
                if !info.ty.is_numeric() && !info.ty.is_opaque() {
                    self.error(
                        INVALID_OPERANDS,
                        format!("Unary '-' expects a numeric operand, found '{}'.", info.ty),
                        span,
                        "Only integers and floats can be negated.",
                    );
                    return ExprInfo::value(Type::Error).with_errors(info.errors);
                }
                ExprInfo::value(info.ty).with_errors(info.errors)
            }
            ast::UnaryOp::Not => {
                let info = self.check_expr(operand, Some(&Type::Bool));
                ExprInfo::value(Type::Bool).with_errors(info.errors)
            }
        }
    }

    fn check_binary(
        &mut self,
        operator: ast::BinaryOp,
        left: &ast::Expr,
        right: &ast::Expr,
        span: Span,
        expected: Option<&Type>,
    ) -> ExprInfo {
        match operator {
            ast::BinaryOp::And | ast::BinaryOp::Or => {
                let left = self.check_expr(left, Some(&Type::Bool));
                let right = self.check_expr(right, Some(&Type::Bool));
                ExprInfo::value(Type::Bool).with_errors(merge_effects(&left.errors, &right.errors))
            }
            ast::BinaryOp::Add
            | ast::BinaryOp::Subtract
            | ast::BinaryOp::Multiply
            | ast::BinaryOp::Divide
            | ast::BinaryOp::Remainder
            | ast::BinaryOp::AddWrap
            | ast::BinaryOp::SubtractWrap
            | ast::BinaryOp::MultiplyWrap
            | ast::BinaryOp::DivideWrap => {
                let numeric_expected = expected.filter(|ty| ty.is_numeric());
                let (left_info, right_info) =
                    if is_numeric_literal(left) && !is_numeric_literal(right) {
                        let right_info = self.check_expr(right, numeric_expected);
                        let left_info = self.check_expr(left, Some(&right_info.ty));
                        (left_info, right_info)
                    } else {
                        let left_info = self.check_expr(left, numeric_expected);
                        let right_info = self.check_expr(right, Some(&left_info.ty));
                        (left_info, right_info)
                    };

                let errors = merge_effects(&left_info.errors, &right_info.errors);
                if left_info.ty == Type::Never || right_info.ty == Type::Never {
                    return ExprInfo {
                        ty: Type::Never,
                        errors,
                    };
                }
                if !left_info.ty.is_numeric() || !right_info.ty.is_numeric() {
                    if !left_info.ty.is_opaque() && !right_info.ty.is_opaque() {
                        self.error(
                            INVALID_OPERANDS,
                            format!(
                                "Operator requires numeric operands, found '{}' and '{}'.",
                                left_info.ty, right_info.ty
                            ),
                            span,
                            "Convert values explicitly; Neo Astra has no implicit numeric coercion.",
                        );
                    }
                    return ExprInfo::value(Type::Error).with_errors(errors);
                }
                if !left_info.ty.accepts(&right_info.ty) && !left_info.ty.is_opaque() {
                    self.type_mismatch(&left_info.ty, &right_info.ty, span);
                }
                ExprInfo::value(left_info.ty).with_errors(errors)
            }
            ast::BinaryOp::Equal | ast::BinaryOp::NotEqual => {
                let left_info = self.check_expr(left, None);
                let right_info = self.check_expr(right, Some(&left_info.ty));
                if !left_info.ty.accepts(&right_info.ty)
                    && !left_info.ty.is_opaque()
                    && right_info.ty != Type::Never
                {
                    self.type_mismatch(&left_info.ty, &right_info.ty, span);
                }
                ExprInfo::value(Type::Bool)
                    .with_errors(merge_effects(&left_info.errors, &right_info.errors))
            }
            ast::BinaryOp::Less
            | ast::BinaryOp::LessEqual
            | ast::BinaryOp::Greater
            | ast::BinaryOp::GreaterEqual => {
                let left_info = self.check_expr(left, None);
                let right_info = self.check_expr(right, Some(&left_info.ty));
                let comparable = left_info.ty.is_numeric()
                    || matches!(left_info.ty, Type::Str | Type::Bytes)
                    || left_info.ty.is_opaque()
                    || left_info.ty == Type::Never;
                if !comparable {
                    self.error(
                        INVALID_OPERANDS,
                        format!("Type '{}' is not ordered.", left_info.ty),
                        span,
                        "Use numeric values, str, or bytes for ordered comparison.",
                    );
                } else if !left_info.ty.accepts(&right_info.ty) && !left_info.ty.is_opaque() {
                    self.type_mismatch(&left_info.ty, &right_info.ty, span);
                }
                ExprInfo::value(Type::Bool)
                    .with_errors(merge_effects(&left_info.errors, &right_info.errors))
            }
        }
    }

    fn check_assignment(&mut self, target: &ast::Expr, value: &ast::Expr, span: Span) -> ExprInfo {
        let target_info = self.check_lvalue(target);
        if !target_info.mutable && !target_info.ty.is_opaque() {
            self.error(
                NOT_ASSIGNABLE,
                "Assignment target is not mutable.",
                target.span,
                "Declare the binding with 'let mut'.",
            );
        }
        let value_info = self.check_expr(value, Some(&target_info.ty));
        let errors = merge_effects(&target_info.errors, &value_info.errors);
        if !target_info.ty.accepts(&value_info.ty) && !target_info.ty.is_opaque() {
            self.type_mismatch(&target_info.ty, &value_info.ty, value.span);
        }
        let _ = span;
        ExprInfo::value(Type::Unit).with_errors(errors)
    }

    fn check_lvalue(&mut self, expression: &ast::Expr) -> LValueInfo {
        match &expression.kind {
            ast::ExprKind::Identifier(name) => {
                if let Some(local) = self.lookup_local(name) {
                    let ty = local.ty.clone();
                    let mutable = local.mutable;
                    self.annotate(expression.span, ty.clone());
                    return LValueInfo {
                        ty,
                        mutable,
                        errors: Vec::new(),
                    };
                }
                if let Some(value) = self.values.get(name) {
                    let ty = value.ty.clone();
                    self.annotate(expression.span, ty.clone());
                    return LValueInfo {
                        ty,
                        mutable: false,
                        errors: Vec::new(),
                    };
                }
                self.error(
                    UNKNOWN_IDENTIFIER,
                    format!("Unknown assignment target '{name}'."),
                    expression.span,
                    "Declare the binding before assigning to it.",
                );
                LValueInfo {
                    ty: Type::Error,
                    mutable: false,
                    errors: Vec::new(),
                }
            }
            ast::ExprKind::Member { target, member } => {
                let base = self.check_lvalue(target);
                let ty = self.member_type(&base.ty, member, expression.span, None);
                self.annotate(expression.span, ty.clone());
                LValueInfo {
                    ty,
                    mutable: base.mutable,
                    errors: base.errors,
                }
            }
            ast::ExprKind::Index { target, index } => {
                let base = self.check_lvalue(target);
                let expected_index = expected_index_type(&base.ty);
                let index_info = self.check_expr(index, expected_index.as_ref());
                let tuple_index = integer_literal_value(index);
                let ty = self.index_type(&base.ty, &index_info.ty, tuple_index, expression.span);
                self.annotate(expression.span, ty.clone());
                LValueInfo {
                    ty,
                    mutable: base.mutable,
                    errors: merge_effects(&base.errors, &index_info.errors),
                }
            }
            _ => {
                self.error(
                    NOT_ASSIGNABLE,
                    "Expression is not a valid assignment target.",
                    expression.span,
                    "Assign to a mutable binding, field, or index.",
                );
                LValueInfo {
                    ty: Type::Error,
                    mutable: false,
                    errors: Vec::new(),
                }
            }
        }
    }

    fn check_call(
        &mut self,
        callee: &ast::Expr,
        arguments: &[ast::Expr],
        span: Span,
        expected: Option<&Type>,
    ) -> ExprInfo {
        if let Some(constructor) = builtin_constructor_name(callee) {
            return self.check_builtin_constructor(constructor, arguments, span, expected);
        }

        let (callee_info, explicit_arguments) = self.check_callee(callee);
        let mut errors = callee_info.errors.clone();
        let function = match callee_info.ty {
            Type::Function(function) => function,
            Type::Unknown => {
                for argument in arguments {
                    let info = self.check_expr(argument, None);
                    append_effects(&mut errors, info.errors);
                }
                return ExprInfo {
                    ty: Type::Unknown,
                    errors,
                };
            }
            Type::Error => {
                for argument in arguments {
                    let info = self.check_expr(argument, None);
                    append_effects(&mut errors, info.errors);
                }
                return ExprInfo {
                    ty: Type::Error,
                    errors,
                };
            }
            other => {
                self.error(
                    NOT_CALLABLE,
                    format!("Type '{other}' is not callable."),
                    span,
                    "Call a function value or a constructor.",
                );
                for argument in arguments {
                    let info = self.check_expr(argument, None);
                    append_effects(&mut errors, info.errors);
                }
                return ExprInfo {
                    ty: Type::Error,
                    errors,
                };
            }
        };

        if function.parameters.len() != arguments.len() {
            self.error(
                ARGUMENT_COUNT,
                format!(
                    "Function expects {} argument(s), but {} were provided.",
                    function.parameters.len(),
                    arguments.len()
                ),
                span,
                "Match the declared function signature.",
            );
        }

        let mut substitutions = BTreeMap::new();
        if let Some(arguments) = &explicit_arguments {
            if arguments.len() != function.type_parameters.len() {
                self.error(
                    INVALID_TYPE_ARGUMENT,
                    format!(
                        "Function expects {} type argument(s), but {} were provided.",
                        function.type_parameters.len(),
                        arguments.len()
                    ),
                    span,
                    "Match the declared type parameter list.",
                );
            }
            for (parameter, argument) in function.type_parameters.iter().zip(arguments) {
                substitutions.insert(parameter.clone(), argument.clone());
            }
        }

        let mut argument_infos = Vec::with_capacity(arguments.len());
        for (index, argument) in arguments.iter().enumerate() {
            let expected_argument = function.parameters.get(index).map(|parameter| {
                if explicit_arguments.is_none()
                    && !function.type_parameters.is_empty()
                    && contains_generic(parameter)
                {
                    None
                } else {
                    Some(parameter.substitute(&substitutions))
                }
            });
            let expected_argument = expected_argument.flatten();
            let info = self.check_expr(argument, expected_argument.as_ref());
            append_effects(&mut errors, info.errors.clone());
            argument_infos.push(info);
        }

        if explicit_arguments.is_none() && !function.type_parameters.is_empty() {
            for (parameter, argument) in function.parameters.iter().zip(&argument_infos) {
                infer_substitutions(parameter, &argument.ty, &mut substitutions);
            }
            if let Some(expected) = expected {
                infer_substitutions(&function.return_type, expected, &mut substitutions);
            }
            for parameter in &function.type_parameters {
                if !substitutions.contains_key(parameter) {
                    self.error(
                        INVALID_TYPE_ARGUMENT,
                        format!("Cannot infer type parameter '{parameter}'."),
                        span,
                        "Add explicit type arguments at the call site.",
                    );
                    substitutions.insert(parameter.clone(), Type::Unknown);
                }
            }
        }

        for (index, (parameter, argument)) in
            function.parameters.iter().zip(&argument_infos).enumerate()
        {
            if index >= arguments.len() {
                break;
            }
            let parameter = parameter.substitute(&substitutions);
            if !parameter.accepts(&argument.ty) && !parameter.is_opaque() {
                self.type_mismatch(&parameter, &argument.ty, arguments[index].span);
            }
        }

        if !function.type_parameters.is_empty() {
            let callee_name = match &callee.kind {
                ast::ExprKind::Identifier(name) => Some(name.clone()),
                ast::ExprKind::TypeArguments { target, .. } => match &target.kind {
                    ast::ExprKind::Identifier(name) => Some(name.clone()),
                    _ => None,
                },
                _ => None,
            };
            if let Some(name) = callee_name {
                let enclosing = self
                    .current_function
                    .as_ref()
                    .map(|f| f.name.clone())
                    .unwrap_or_default();
                self.pending_generic_call_checks.push((
                    span,
                    name,
                    enclosing,
                    substitutions.clone(),
                ));
            }
        }

        let return_type = function.return_type.substitute(&substitutions);
        let function_errors = function
            .errors
            .iter()
            .map(|error| ErrorEffect::new(error.substitute(&substitutions), span))
            .collect::<Vec<_>>();
        append_effects(&mut errors, function_errors);

        self.check_manual_alloc_element(callee, &return_type, span);
        self.check_channel_element(callee, &return_type, span);

        ExprInfo {
            ty: return_type,
            errors,
        }
    }

    fn check_callee(&mut self, callee: &ast::Expr) -> (ExprInfo, Option<Vec<Type>>) {
        if let ast::ExprKind::TypeArguments { target, arguments } = &callee.kind {
            let generics = generic_map(&self.current_type_parameters);
            let explicit = arguments
                .iter()
                .map(|argument| self.resolve_type(argument, &generics))
                .collect::<Vec<_>>();
            let info = self.check_expr(target, None);
            return (info, Some(explicit));
        }

        (self.check_expr(callee, None), None)
    }

    fn check_index(&mut self, target: &ast::Expr, index: &ast::Expr, span: Span) -> ExprInfo {
        let target_info = self.check_expr(target, None);
        let expected_index = expected_index_type(&target_info.ty);
        let index_info = self.check_expr(index, expected_index.as_ref());
        let tuple_index = integer_literal_value(index);
        let ty = self.index_type(&target_info.ty, &index_info.ty, tuple_index, span);
        ExprInfo {
            ty,
            errors: merge_effects(&target_info.errors, &index_info.errors),
        }
    }

    fn index_type(
        &mut self,
        target: &Type,
        index: &Type,
        tuple_index: Option<usize>,
        span: Span,
    ) -> Type {
        match target {
            Type::Array(element) | Type::Slice(element) | Type::FixedArray(element, _) => {
                if !index.accepts(&Type::Usize) && !index.is_opaque() {
                    self.type_mismatch(&Type::Usize, index, span);
                }
                (**element).clone()
            }
            Type::Bytes => {
                if !index.accepts(&Type::Usize) && !index.is_opaque() {
                    self.type_mismatch(&Type::Usize, index, span);
                }
                Type::U8
            }
            Type::Map(key, value) => {
                if !key.accepts(index) && !key.is_opaque() {
                    self.type_mismatch(key, index, span);
                }
                (**value).clone()
            }
            Type::Tuple(elements) => {
                if let Some(index) = tuple_index
                    && let Some(element) = elements.get(index)
                {
                    return element.clone();
                }
                self.error(
                    INVALID_MEMBER,
                    "Tuple index is invalid.".to_owned(),
                    span,
                    format!("Use an integer index in 0..{}.", elements.len()),
                );
                Type::Error
            }
            Type::Unknown | Type::Error => Type::Unknown,
            _ => {
                self.error(
                    INVALID_MEMBER,
                    format!("Type '{target}' cannot be indexed."),
                    span,
                    "Use an array, slice, map, or tuple.",
                );
                Type::Error
            }
        }
    }

    fn check_member(
        &mut self,
        target: &ast::Expr,
        member: &str,
        span: Span,
        expected: Option<&Type>,
    ) -> ExprInfo {
        if let ast::ExprKind::Identifier(name) = &target.kind
            && self.lookup_local(name).is_none()
        {
            if let Some(enum_def) = self.resolve_enum(name) {
                return self.enum_member(name, &enum_def, member, span, expected);
            }
            if let Some(namespace) = self.namespaces.get(name).cloned() {
                if let Some(&script_mode) = self.script_modes.get(name)
                    && !self.mode.allows(script_mode)
                {
                    self.error(
                        MODE_REQUIREMENT,
                        format!(
                            "Script block '{name}' requires '{}' mode, but the active mode is '{}'.",
                            script_mode.as_str(),
                            self.mode.as_str()
                        ),
                        span,
                        format!(
                            "Move the call into a {}-mode function or raise the mode.",
                            script_mode.as_str()
                        ),
                    );
                }
                if let Some(value) = namespace.members.get(member) {
                    return ExprInfo::value(value.ty.clone());
                }
                if namespace.open {
                    return ExprInfo::value(Type::Unknown);
                }
                self.error(
                    INVALID_MEMBER,
                    format!("Foreign module '{name}' has no member '{member}'."),
                    span,
                    "Declare the function inside the foreign module block.",
                );
                return ExprInfo::value(Type::Error);
            }

            if member == "create" && name == "Channel" && self.resolve_struct(name).is_none() {
                return ExprInfo::value(Type::Function(FunctionType {
                    type_parameters: vec!["T".to_owned()],
                    parameters: Vec::new(),
                    return_type: Box::new(Type::Named(
                        "Channel".to_owned(),
                        vec![Type::Generic("T".to_owned())],
                    )),
                    errors: Vec::new(),
                }));
            }
            if member == "new" && name == "bytes" && self.resolve_struct(name).is_none() {
                return ExprInfo::value(Type::Function(FunctionType {
                    type_parameters: Vec::new(),
                    parameters: vec![Type::Usize],
                    return_type: Box::new(Type::Bytes),
                    errors: Vec::new(),
                }));
            }

            if name == "Memory" && self.resolve_struct(name).is_none() {
                if !self.mode.allows(Mode::System) {
                    self.error(
                        MEMORY_REQUIREMENT,
                        format!(
                            "Manual memory requires system mode, but the active mode is '{}'.",
                            self.mode.as_str()
                        ),
                        span,
                        "Use 'Memory' only inside a system-mode boundary.",
                    );
                    return ExprInfo::value(Type::Error);
                }
                if member == "alloc" {
                    if !self.in_manual_memory_scope(span) {
                        self.error(
                            MANUAL_ALLOC_IN_GC_SCOPE,
                            "Memory.alloc is only allowed inside an '@memory:manual' scope, \
                             but this expression is in a GC-managed scope.",
                            span,
                            "Wrap this statement in {@memory:manual; ...} inside a system-mode boundary.",
                        );
                        return ExprInfo::value(Type::Error);
                    }

                    self.manual_alloc_spans.push(span);
                    return ExprInfo::value(Type::Function(FunctionType {
                        type_parameters: vec!["T".to_owned()],
                        parameters: vec![Type::Usize],
                        return_type: Box::new(Type::Named(
                            "Ptr".to_owned(),
                            vec![Type::Generic("T".to_owned())],
                        )),
                        errors: Vec::new(),
                    }));
                }
                if member == "free" {
                    return ExprInfo::value(Type::Function(FunctionType {
                        type_parameters: vec!["T".to_owned()],
                        parameters: vec![Type::Named(
                            "Ptr".to_owned(),
                            vec![Type::Generic("T".to_owned())],
                        )],
                        return_type: Box::new(Type::Unit),
                        errors: Vec::new(),
                    }));
                }
            }

            if name == "Region" && member == "new" && self.resolve_struct(name).is_none() {
                if !self.mode.allows(Mode::System) {
                    self.error(
                        MEMORY_REQUIREMENT,
                        format!(
                            "Manual memory requires system mode, but the active mode is '{}'.",
                            self.mode.as_str()
                        ),
                        span,
                        "Use 'Region' only inside a system-mode boundary.",
                    );
                    return ExprInfo::value(Type::Error);
                }
                return ExprInfo::value(Type::Function(FunctionType {
                    type_parameters: Vec::new(),
                    parameters: vec![Type::Usize],
                    return_type: Box::new(Type::Named("Region".to_owned(), Vec::new())),
                    errors: Vec::new(),
                }));
            }
        }

        let target_info = self.check_expr(target, None);

        if member == "alloc" && matches!(&target_info.ty, Type::Named(name, _) if name == "Region")
        {
            self.manual_alloc_spans.push(span);
        }
        let ty = self.member_type(&target_info.ty, member, target.span, expected);
        ExprInfo {
            ty,
            errors: target_info.errors,
        }
    }

    fn enum_member(
        &mut self,
        enum_name: &str,
        enum_def: &ResolvedEnum,
        member: &str,
        span: Span,
        expected: Option<&Type>,
    ) -> ExprInfo {
        let Some(variant) = enum_def
            .variants
            .iter()
            .find(|variant| variant.name == member)
            .cloned()
        else {
            self.error(
                INVALID_MEMBER,
                format!("Enum '{enum_name}' has no variant '{member}'."),
                span,
                "Use a declared variant name.",
            );
            return ExprInfo::value(Type::Error);
        };

        let substitutions = generic_map(&enum_def.type_parameters);
        let generic_return = Type::Named(
            enum_name.to_owned(),
            enum_def
                .type_parameters
                .iter()
                .map(|parameter| Type::Generic(parameter.clone()))
                .collect(),
        );
        let concrete_return = expected
            .cloned()
            .filter(|ty| matches!(ty, Type::Named(name, _) if name == enum_name));

        match variant.fields {
            Some(fields) => {
                let parameters = fields
                    .iter()
                    .map(|field| field.ty.substitute(&substitutions))
                    .collect::<Vec<_>>();
                ExprInfo::value(Type::Function(FunctionType {
                    type_parameters: enum_def.type_parameters.clone(),
                    parameters,
                    return_type: Box::new(generic_return),
                    errors: Vec::new(),
                }))
            }
            None => ExprInfo::value(concrete_return.unwrap_or(generic_return)),
        }
    }

    fn member_type(
        &mut self,
        ty: &Type,
        member: &str,
        span: Span,
        expected: Option<&Type>,
    ) -> Type {
        match ty {
            Type::Named(name, arguments) => {
                if let Some(struct_def) = self.resolve_struct(name) {
                    let substitutions = substitutions(&struct_def.type_parameters, arguments);
                    if let Some(field) = struct_def
                        .fields
                        .iter()
                        .find(|field| field.name.as_deref() == Some(member))
                    {
                        return field.ty.substitute(&substitutions);
                    }
                    if let Some(method) = self.struct_method(name, member, ty, span) {
                        return method;
                    }
                    self.error(
                        INVALID_MEMBER,
                        format!("Struct '{name}' has no field or method '{member}'."),
                        span,
                        "Check the field name or the struct declaration.",
                    );
                    return Type::Error;
                }

                if name == "Ptr"
                    && matches!(member, "read" | "write")
                    && !self.mode.allows(Mode::System)
                {
                    self.error(
                        MEMORY_REQUIREMENT,
                        format!(
                            "Manual memory requires system mode, but the active mode is '{}'.",
                            self.mode.as_str()
                        ),
                        span,
                        "Use pointer access only inside a system-mode boundary.",
                    );
                    return Type::Error;
                }

                if name == "Region"
                    && matches!(member, "alloc" | "free")
                    && !self.mode.allows(Mode::System)
                {
                    self.error(
                        MEMORY_REQUIREMENT,
                        format!(
                            "Manual memory requires system mode, but the active mode is '{}'.",
                            self.mode.as_str()
                        ),
                        span,
                        "Use region allocation only inside a system-mode boundary.",
                    );
                    return Type::Error;
                }
                if let Some(method) = builtin_method(ty, member) {
                    return method;
                }
                if ty.is_opaque() {
                    return Type::Unknown;
                }
                let _ = expected;
                self.error(
                    INVALID_MEMBER,
                    format!("Type '{ty}' has no member '{member}'."),
                    span,
                    "Check the member name.",
                );
                Type::Error
            }
            Type::Tuple(elements) => {
                if let Ok(index) = member.parse::<usize>()
                    && let Some(element) = elements.get(index)
                {
                    return element.clone();
                }
                self.error(
                    INVALID_MEMBER,
                    "Tuple index is invalid.".to_owned(),
                    span,
                    format!("Use an index in 0..{}.", elements.len()),
                );
                Type::Error
            }
            Type::Unknown | Type::Error => Type::Unknown,
            _ => {
                if let Some(method) = builtin_method(ty, member) {
                    return method;
                }
                self.error(
                    INVALID_MEMBER,
                    format!("Type '{ty}' has no member '{member}'."),
                    span,
                    "Check the member name or add a type annotation.",
                );
                Type::Error
            }
        }
    }

    fn struct_method(
        &mut self,
        _name: &str,
        _member: &str,
        _ty: &Type,
        _span: Span,
    ) -> Option<Type> {
        None
    }

    fn check_builtin_constructor(
        &mut self,
        constructor: &str,
        arguments: &[ast::Expr],
        span: Span,
        expected: Option<&Type>,
    ) -> ExprInfo {
        match constructor {
            "Some" => {
                let argument_expected = match expected {
                    Some(Type::Option(inner)) => Some(&**inner),
                    _ => None,
                };
                if arguments.len() != 1 {
                    self.error(
                        ARGUMENT_COUNT,
                        "Some expects exactly one argument.",
                        span,
                        "Use Some(value).",
                    );
                }
                let mut errors = Vec::new();
                let mut value_type = Type::Never;
                for (index, argument) in arguments.iter().enumerate() {
                    let expected = if index == 0 { argument_expected } else { None };
                    let info = self.check_expr(argument, expected);
                    if index == 0 {
                        value_type = info.ty;
                    }
                    append_effects(&mut errors, info.errors);
                }
                ExprInfo {
                    ty: Type::Option(Box::new(value_type)),
                    errors,
                }
            }
            "Ok" | "Err" => {
                let (ok_expected, err_expected) = match expected {
                    Some(Type::Result(ok, error)) => (Some(&**ok), Some(&**error)),
                    _ => (None, None),
                };
                if arguments.len() != 1 {
                    self.error(
                        ARGUMENT_COUNT,
                        format!("{constructor} expects exactly one argument."),
                        span,
                        format!("Use {constructor}(value)."),
                    );
                }
                let expected_argument = if constructor == "Ok" {
                    ok_expected
                } else {
                    err_expected
                };
                let mut errors = Vec::new();
                let mut value_type = Type::Never;
                for (index, argument) in arguments.iter().enumerate() {
                    let info = self
                        .check_expr(argument, if index == 0 { expected_argument } else { None });
                    if index == 0 {
                        value_type = info.ty;
                    }
                    append_effects(&mut errors, info.errors);
                }
                let ty = if constructor == "Ok" {
                    Type::Result(Box::new(value_type), Box::new(Type::Never))
                } else {
                    Type::Result(Box::new(Type::Never), Box::new(value_type))
                };
                ExprInfo { ty, errors }
            }
            _ => {
                self.error(
                    NOT_CALLABLE,
                    format!("Unknown built-in constructor '{constructor}'."),
                    span,
                    "Use Some, Ok, or Err.",
                );
                ExprInfo::value(Type::Error)
            }
        }
    }

    fn check_propagation(&mut self, inner: &ast::Expr, span: Span) -> ExprInfo {
        let info = self.check_expr(inner, None);
        if !info.errors.is_empty() {
            for error in &info.errors {
                self.record_error(error);
            }
            return ExprInfo::value(info.ty);
        }

        if let Type::Result(ok, error) = info.ty {
            self.record_error(&ErrorEffect::new((*error).clone(), span));
            return ExprInfo::value((*ok).clone());
        }

        if !info.ty.is_opaque() {
            self.error(
                INVALID_PROPAGATION,
                format!("Expression of type '{}' is not fallible.", info.ty),
                span,
                "Use '!' only on fallible calls or Result values.",
            );
        }
        ExprInfo::value(info.ty)
    }

    fn check_array(
        &mut self,
        elements: &[ast::Expr],
        span: Span,
        expected: Option<&Type>,
    ) -> ExprInfo {
        let (element_expected, fixed_length) = match expected {
            Some(Type::Array(inner)) | Some(Type::Slice(inner)) => (Some(&**inner), None),
            Some(Type::FixedArray(inner, length)) => (Some(&**inner), Some(*length)),
            _ => (None, None),
        };

        if let Some(length) = fixed_length
            && length != elements.len()
        {
            self.error(
                TYPE_MISMATCH,
                format!(
                    "Fixed array expects {length} element(s), but {} were provided.",
                    elements.len()
                ),
                span,
                "Match the declared fixed-array length.",
            );
        }

        let mut element_type = element_expected.cloned().unwrap_or(Type::Never);
        let mut errors = Vec::new();
        for element in elements {
            let expected_element = if element_type == Type::Never {
                element_expected
            } else {
                Some(&element_type)
            };
            let info = self.check_expr(element, expected_element);
            append_effects(&mut errors, info.errors);
            if element_type == Type::Never {
                element_type = info.ty.clone();
            } else if !element_type.accepts(&info.ty) && !element_type.is_opaque() {
                self.type_mismatch(&element_type, &info.ty, element.span);
            }
        }

        let ty = match fixed_length {
            Some(length) => Type::FixedArray(Box::new(element_type), length),
            None => Type::Array(Box::new(element_type)),
        };
        ExprInfo { ty, errors }
    }

    fn check_map(
        &mut self,
        entries: &[ast::MapEntry],
        _span: Span,
        expected: Option<&Type>,
    ) -> ExprInfo {
        let (key_expected, value_expected) = match expected {
            Some(Type::Map(key, value)) => (Some(&**key), Some(&**value)),
            _ => (None, None),
        };

        let mut key_type = key_expected.cloned().unwrap_or(Type::Never);
        let mut value_type = value_expected.cloned().unwrap_or(Type::Never);
        let mut errors = Vec::new();

        for entry in entries {
            let key_expected_item = if key_type == Type::Never {
                key_expected
            } else {
                Some(&key_type)
            };
            let value_expected_item = if value_type == Type::Never {
                value_expected
            } else {
                Some(&value_type)
            };
            let key = self.check_expr(&entry.key, key_expected_item);
            let value = self.check_expr(&entry.value, value_expected_item);
            append_effects(&mut errors, key.errors);
            append_effects(&mut errors, value.errors);
            if key_type == Type::Never {
                key_type = key.ty.clone();
            } else if !key_type.accepts(&key.ty) && !key_type.is_opaque() {
                self.type_mismatch(&key_type, &key.ty, entry.key.span);
            }
            if value_type == Type::Never {
                value_type = value.ty.clone();
            } else if !value_type.accepts(&value.ty) && !value_type.is_opaque() {
                self.type_mismatch(&value_type, &value.ty, entry.value.span);
            }
        }

        ExprInfo {
            ty: Type::Map(Box::new(key_type), Box::new(value_type)),
            errors,
        }
    }

    fn check_tuple(
        &mut self,
        elements: &[ast::Expr],
        _span: Span,
        expected: Option<&Type>,
    ) -> ExprInfo {
        let expected_elements = match expected {
            Some(Type::Tuple(elements)) => Some(elements.as_slice()),
            _ => None,
        };
        if let Some(expected_elements) = expected_elements
            && expected_elements.len() != elements.len()
        {
            self.error(
                TYPE_MISMATCH,
                format!(
                    "Tuple expects {} element(s), but {} were provided.",
                    expected_elements.len(),
                    elements.len()
                ),
                _span,
                "Match the declared tuple type.",
            );
        }

        let mut types = Vec::with_capacity(elements.len());
        let mut errors = Vec::new();
        for (index, element) in elements.iter().enumerate() {
            let expected_element = expected_elements.and_then(|elements| elements.get(index));
            let info = self.check_expr(element, expected_element);
            append_effects(&mut errors, info.errors);
            types.push(info.ty);
        }

        ExprInfo {
            ty: Type::Tuple(types),
            errors,
        }
    }

    fn check_struct_init(
        &mut self,
        path: &ast::TypePath,
        fields: &[ast::FieldInit],
        span: Span,
    ) -> ExprInfo {
        let arguments = path
            .arguments
            .iter()
            .map(|argument| self.resolve_type(argument, &BTreeMap::new()))
            .collect::<Vec<_>>();
        let name = path
            .segments
            .last()
            .cloned()
            .unwrap_or_else(|| path.segments.join("."));

        let Some(struct_def) = self.resolve_struct(&name) else {
            if self.imported_types.contains(&name) {
                let mut errors = Vec::new();
                for field in fields {
                    let info = self.check_expr(&field.value, None);
                    append_effects(&mut errors, info.errors);
                }
                return ExprInfo {
                    ty: Type::Unknown,
                    errors,
                };
            }
            self.error(
                UNKNOWN_TYPE,
                format!("Unknown struct type '{name}'."),
                span,
                "Declare the struct before constructing it.",
            );
            return ExprInfo::value(Type::Error);
        };

        if !check_arity(&struct_def.type_parameters, &arguments, &name, span, self) {
            return ExprInfo::value(Type::Error);
        }

        let substitutions = substitutions(&struct_def.type_parameters, &arguments);
        let mut errors = Vec::new();
        let mut seen = BTreeSet::new();

        for field in fields {
            if !seen.insert(field.name.clone()) {
                self.error(
                    INVALID_STRUCT_FIELD,
                    format!("Field '{}' is initialized more than once.", field.name),
                    field.span,
                    "Remove the duplicate initializer.",
                );
            }
            let declared = struct_def
                .fields
                .iter()
                .find(|candidate| candidate.name.as_deref() == Some(field.name.as_str()));
            match declared {
                Some(declared) => {
                    let expected = declared.ty.substitute(&substitutions);
                    let info = self.check_expr(&field.value, Some(&expected));
                    append_effects(&mut errors, info.errors);
                }
                None => {
                    self.error(
                        INVALID_STRUCT_FIELD,
                        format!("Struct '{name}' has no field '{}'.", field.name),
                        field.span,
                        "Use a declared field name.",
                    );
                    let info = self.check_expr(&field.value, None);
                    append_effects(&mut errors, info.errors);
                }
            }
        }

        for field in &struct_def.fields {
            if let Some(field_name) = &field.name
                && !seen.contains(field_name)
            {
                self.error(
                    INVALID_STRUCT_FIELD,
                    format!("Field '{field_name}' is missing from the initializer."),
                    span,
                    "Initialize every field of the struct.",
                );
            }
        }

        ExprInfo {
            ty: Type::Named(name, arguments),
            errors,
        }
    }

    fn check_block_expression(&mut self, block: &ast::Block, expected: Option<&Type>) -> ExprInfo {
        let info = self.check_block(block, true, expected);
        let (ty, errors) = if info.diverges {
            (Type::Never, Vec::new())
        } else {
            match info.tail {
                Some(tail) => (tail.ty, tail.errors),
                None => (Type::Unit, Vec::new()),
            }
        };
        ExprInfo { ty, errors }
    }

    fn check_if(
        &mut self,
        condition: &ast::Expr,
        then_branch: &ast::Block,
        else_branch: Option<&ast::ElseBranch>,
        span: Span,
        expected: Option<&Type>,
    ) -> ExprInfo {
        let condition_info = self.check_expr(condition, Some(&Type::Bool));
        let then_info = self.check_block(then_branch, true, expected);
        let then_type = block_value_type(&then_info);
        let then_errors = block_tail_errors(&then_info);

        let (else_type, else_errors) = match else_branch {
            Some(ast::ElseBranch::Block(block)) => {
                let info = self.check_block(block, true, expected.or(Some(&then_type)));
                (block_value_type(&info), block_tail_errors(&info))
            }
            Some(ast::ElseBranch::If(expression)) => {
                let info = self.check_expr(expression, expected.or(Some(&then_type)));
                (info.ty, info.errors)
            }
            None => (Type::Unit, Vec::new()),
        };

        let ty = if then_type == Type::Never && else_type == Type::Never {
            Type::Never
        } else if then_type.accepts(&else_type) {
            then_type
        } else if else_type.accepts(&then_type) {
            else_type
        } else {
            self.type_mismatch(&then_type, &else_type, span);
            Type::Error
        };

        ExprInfo {
            ty,
            errors: merge_effects(
                &merge_effects(&condition_info.errors, &then_errors),
                &else_errors,
            ),
        }
    }

    fn check_match(
        &mut self,
        value: &ast::Expr,
        arms: &[ast::MatchArm],
        span: Span,
        expected: Option<&Type>,
    ) -> ExprInfo {
        let scrutinee = self.check_expr(value, None);
        let effects = scrutinee.errors.clone();
        let effect_types = effects
            .iter()
            .map(|effect| effect.ty.clone())
            .collect::<Vec<_>>();
        let shape = self.match_shape(&scrutinee.ty, &effect_types);

        let mut coverage = Coverage::default();
        let mut result_type: Option<Type> = expected.cloned();
        let mut diverges = !arms.is_empty();
        let mut errors = Vec::new();

        for arm in arms {
            self.push_scope();
            self.check_pattern(&arm.pattern, &scrutinee.ty, &effect_types, &mut coverage);
            let expected_arm = result_type.as_ref();
            let (arm_type, arm_errors) = match &arm.body {
                ast::MatchArmBody::Expr(expression) => {
                    let info = self.check_expr(expression, expected_arm);
                    (info.ty, info.errors)
                }
                ast::MatchArmBody::Block(block) => {
                    let info = self.check_block(block, true, expected_arm);
                    (block_value_type(&info), block_tail_errors(&info))
                }
            };
            self.pop_scope();
            append_effects(&mut errors, arm_errors);

            if arm_type != Type::Never {
                diverges = false;
                if let Some(result_type) = &mut result_type {
                    if !result_type.accepts(&arm_type) && !result_type.is_opaque() {
                        self.type_mismatch(result_type, &arm_type, arm.span);
                    }
                } else {
                    result_type = Some(arm_type);
                }
            }
        }

        let scrutinee_type =
            if effect_types.is_empty() || matches!(scrutinee.ty, Type::Result(_, _)) {
                scrutinee.ty.clone()
            } else {
                let error = if effect_types.len() == 1 {
                    effect_types[0].clone()
                } else {
                    Type::Unknown
                };
                Type::Result(Box::new(scrutinee.ty.clone()), Box::new(error))
            };
        let patterns = arms.iter().map(|arm| &arm.pattern).collect::<Vec<_>>();
        if !self.patterns_are_exhaustive(&patterns, &scrutinee_type) {
            let missing = coverage.missing(&shape);
            self.error(
                NON_EXHAUSTIVE_MATCH,
                format!("Non-exhaustive match: missing {}.", missing.join(", ")),
                span,
                "Add the missing case(s) or a wildcard arm.",
            );
        }

        let ty = if diverges {
            Type::Never
        } else {
            result_type.unwrap_or(Type::Unit)
        };
        ExprInfo { ty, errors }
    }

    fn patterns_are_exhaustive(&mut self, patterns: &[&ast::Pattern], ty: &Type) -> bool {
        let rows = patterns
            .iter()
            .map(|pattern| vec![RowPattern::Pattern(pattern)])
            .collect::<Vec<_>>();
        self.rows_are_exhaustive(&rows, std::slice::from_ref(ty))
    }

    fn rows_are_exhaustive(&mut self, rows: &[Vec<RowPattern<'_>>], types: &[Type]) -> bool {
        if types.is_empty() {
            return !rows.is_empty();
        }
        if rows.is_empty() {
            return false;
        }
        let ty = &types[0];
        if matches!(
            ty,
            Type::Never | Type::Unknown | Type::Error | Type::Generic(_)
        ) {
            return true;
        }

        let normalized = rows
            .iter()
            .map(|row| {
                let mut row = row.to_vec();
                if let Some(first) = row.first_mut()
                    && let RowPattern::Pattern(pattern) = first
                    && pattern_is_catch_all(pattern, ty)
                {
                    *first = RowPattern::Wildcard;
                }
                row
            })
            .collect::<Vec<_>>();
        let rows = &normalized;

        if rows
            .iter()
            .all(|row| matches!(row.first(), Some(RowPattern::Wildcard)))
        {
            let next = rows.iter().map(|row| row[1..].to_vec()).collect::<Vec<_>>();
            return self.rows_are_exhaustive(&next, &types[1..]);
        }

        let constructors = self.constructors_for(ty);
        let remaining_types = &types[1..];
        if constructors.is_empty() {
            let next = rows
                .iter()
                .filter(|row| matches!(row.first(), Some(RowPattern::Wildcard)))
                .map(|row| row[1..].to_vec())
                .collect::<Vec<_>>();
            return self.rows_are_exhaustive(&next, remaining_types);
        }

        for constructor in constructors {
            let mut next_rows = Vec::new();
            for row in rows {
                match row.first() {
                    Some(RowPattern::Wildcard) => {
                        let mut expanded = vec![RowPattern::Wildcard; constructor.fields.len()];
                        expanded.extend(row[1..].iter().copied());
                        next_rows.push(expanded);
                    }
                    Some(RowPattern::Pattern(pattern)) => {
                        if let Some(mut expanded) =
                            pattern_row(pattern, ty, &constructor.key, constructor.fields.len())
                        {
                            expanded.extend(row[1..].iter().copied());
                            next_rows.push(expanded);
                        }
                    }
                    None => {}
                }
            }

            let mut next_types = constructor.fields.clone();
            next_types.extend(remaining_types.iter().cloned());
            if !self.rows_are_exhaustive(&next_rows, &next_types) {
                return false;
            }
        }

        true
    }

    fn constructors_for(&mut self, ty: &Type) -> Vec<Constructor> {
        match ty {
            Type::Option(inner) => vec![
                Constructor {
                    key: ConstructorKey::Some,
                    fields: vec![(**inner).clone()],
                },
                Constructor {
                    key: ConstructorKey::None,
                    fields: Vec::new(),
                },
            ],
            Type::Result(ok, error) => vec![
                Constructor {
                    key: ConstructorKey::Ok,
                    fields: vec![(**ok).clone()],
                },
                Constructor {
                    key: ConstructorKey::Err,
                    fields: vec![(**error).clone()],
                },
            ],
            Type::Bool => vec![
                Constructor {
                    key: ConstructorKey::Bool(false),
                    fields: Vec::new(),
                },
                Constructor {
                    key: ConstructorKey::Bool(true),
                    fields: Vec::new(),
                },
            ],
            Type::Tuple(elements) => vec![Constructor {
                key: ConstructorKey::Tuple,
                fields: elements.clone(),
            }],
            Type::Unit => vec![Constructor {
                key: ConstructorKey::Unit,
                fields: Vec::new(),
            }],
            Type::Named(name, arguments) => {
                let Some(enum_def) = self.resolve_enum(name) else {
                    return Vec::new();
                };
                let substitutions = substitutions(&enum_def.type_parameters, arguments);
                enum_def
                    .variants
                    .iter()
                    .map(|variant| Constructor {
                        key: ConstructorKey::Variant(variant.name.clone()),
                        fields: variant
                            .fields
                            .as_ref()
                            .map(|fields| {
                                fields
                                    .iter()
                                    .map(|field| field.ty.substitute(&substitutions))
                                    .collect()
                            })
                            .unwrap_or_default(),
                    })
                    .collect()
            }
            _ => Vec::new(),
        }
    }

    fn match_shape(&mut self, ty: &Type, effects: &[Type]) -> MatchShape {
        if !effects.is_empty() {
            return MatchShape::Result;
        }
        match ty {
            Type::Option(_) => MatchShape::Option,
            Type::Result(_, _) => MatchShape::Result,
            Type::Bool => MatchShape::Bool,
            Type::Tuple(_) => MatchShape::Tuple,
            Type::Named(name, _) => {
                if let Some(enum_def) = self.resolve_enum(name) {
                    MatchShape::Enum(enum_def)
                } else {
                    MatchShape::Other
                }
            }
            _ => MatchShape::Other,
        }
    }

    fn check_pattern(
        &mut self,
        pattern: &ast::Pattern,
        ty: &Type,
        effects: &[Type],
        coverage: &mut Coverage,
    ) {
        match &pattern.kind {
            ast::PatternKind::Wildcard => {}
            ast::PatternKind::Identifier(name) => {
                if name == "None" && matches!(ty, Type::Option(_)) {
                    coverage.none = true;
                    return;
                }
                self.insert_local(
                    name.clone(),
                    Local {
                        ty: ty.clone(),
                        mutable: false,
                        span: pattern.span,
                    },
                );
            }
            ast::PatternKind::Literal(literal) => {
                let literal_type = self.literal_type(literal, Some(ty), pattern.span);
                if !ty.accepts(&literal_type) && !ty.is_opaque() {
                    self.type_mismatch(ty, &literal_type, pattern.span);
                }
                if let ast::Literal::Bool(value) = literal {
                    coverage.boolean[usize::from(*value)] = true;
                }
            }
            ast::PatternKind::Tuple(patterns) => {
                let Type::Tuple(elements) = ty else {
                    if !ty.is_opaque() {
                        self.error(
                            INVALID_PATTERN,
                            format!("Tuple pattern cannot match type '{ty}'."),
                            pattern.span,
                            "Use a tuple pattern only for tuple values.",
                        );
                    }
                    return;
                };
                if patterns.len() != elements.len() {
                    self.error(
                        INVALID_PATTERN,
                        format!(
                            "Tuple pattern has {} element(s), but type '{ty}' has {}.",
                            patterns.len(),
                            elements.len()
                        ),
                        pattern.span,
                        "Match every tuple element.",
                    );
                }
                for (child, element) in patterns.iter().zip(elements) {
                    let mut nested = Coverage::default();
                    self.check_pattern(child, element, &[], &mut nested);
                }
            }
            ast::PatternKind::Constructor { path, arguments } => {
                let name = path
                    .segments
                    .last()
                    .cloned()
                    .unwrap_or_else(|| path.segments.join("."));
                match name.as_str() {
                    "Some" => {
                        coverage.some = true;
                        let Type::Option(inner) = ty else {
                            if !ty.is_opaque() {
                                self.error(
                                    INVALID_PATTERN,
                                    format!("Pattern 'Some' cannot match type '{ty}'."),
                                    pattern.span,
                                    "Use Some only for Option<T> values.",
                                );
                            }
                            return;
                        };
                        self.check_pattern_arguments(
                            arguments,
                            std::slice::from_ref(&**inner),
                            pattern.span,
                            coverage,
                        );
                    }
                    "Ok" => {
                        coverage.ok = true;
                        let (ok, _) = result_parts(ty, effects);
                        self.check_pattern_arguments(arguments, &[ok], pattern.span, coverage);
                    }
                    "Err" => {
                        coverage.err = true;
                        let (_, error) = result_parts(ty, effects);
                        self.check_pattern_arguments(arguments, &[error], pattern.span, coverage);
                    }
                    _ => {
                        self.error(
                            INVALID_PATTERN,
                            format!("Unknown constructor pattern '{name}'."),
                            pattern.span,
                            "Use Some, Ok, Err, or a declared enum variant.",
                        );
                    }
                }
            }
            ast::PatternKind::Variant {
                path,
                variant,
                arguments,
            } => {
                let enum_name = path
                    .segments
                    .last()
                    .cloned()
                    .unwrap_or_else(|| path.segments.join("."));
                if enum_name == "Option" || enum_name == "Result" {
                    let mut constructor_path = path.clone();
                    constructor_path.segments = vec![variant.clone()];
                    let constructor = ast::Pattern {
                        kind: ast::PatternKind::Constructor {
                            path: constructor_path,
                            arguments: arguments.clone().unwrap_or_default(),
                        },
                        span: pattern.span,
                    };
                    self.check_pattern(&constructor, ty, effects, coverage);
                    return;
                }

                let Some(enum_def) = self.resolve_enum(&enum_name) else {
                    self.error(
                        UNKNOWN_TYPE,
                        format!("Unknown enum type '{enum_name}'."),
                        pattern.span,
                        "Declare the enum before matching on it.",
                    );
                    return;
                };
                let Some(variant_def) = enum_def
                    .variants
                    .iter()
                    .find(|candidate| candidate.name == *variant)
                    .cloned()
                else {
                    self.error(
                        INVALID_PATTERN,
                        format!("Enum '{enum_name}' has no variant '{variant}'."),
                        pattern.span,
                        "Use a declared variant name.",
                    );
                    return;
                };
                coverage.variants.insert(variant.clone());

                let Type::Named(actual_name, actual_arguments) = ty else {
                    if !ty.is_opaque() {
                        self.error(
                            INVALID_PATTERN,
                            format!("Variant pattern cannot match type '{ty}'."),
                            pattern.span,
                            "Use a variant pattern only for enum values.",
                        );
                    }
                    return;
                };
                if actual_name != &enum_name {
                    self.type_mismatch(ty, &Type::Named(enum_name, Vec::new()), pattern.span);
                    return;
                }
                let substitutions = substitutions(&enum_def.type_parameters, actual_arguments);
                let field_types = variant_def
                    .fields
                    .unwrap_or_default()
                    .iter()
                    .map(|field| field.ty.substitute(&substitutions))
                    .collect::<Vec<_>>();
                self.check_pattern_arguments(
                    arguments.as_deref().unwrap_or(&[]),
                    &field_types,
                    pattern.span,
                    coverage,
                );
            }
        }
    }

    fn check_pattern_arguments(
        &mut self,
        patterns: &[ast::Pattern],
        types: &[Type],
        span: Span,
        _coverage: &mut Coverage,
    ) {
        if patterns.len() != types.len() {
            self.error(
                INVALID_PATTERN,
                format!(
                    "Pattern binds {} value(s), but the constructor has {} field(s).",
                    patterns.len(),
                    types.len()
                ),
                span,
                "Match every constructor field.",
            );
        }
        for (pattern, ty) in patterns.iter().zip(types) {
            let mut nested = Coverage::default();
            self.check_pattern(pattern, ty, &[], &mut nested);
        }
    }

    fn check_fail(
        &mut self,
        error: &ast::TypePath,
        arguments: &ast::FailArguments,
        span: Span,
    ) -> ExprInfo {
        let error_type = self.resolve_type_path(error, &BTreeMap::new());
        if !matches!(error_type, Type::Named(_, _) | Type::Unknown | Type::Error) {
            self.error(
                UNDECLARED_ERROR,
                format!("Failure type '{error_type}' is not nominal."),
                span,
                "Use a struct or enum error type.",
            );
        }
        self.record_error(&ErrorEffect::new(error_type.clone(), span));

        let mut errors = Vec::new();
        match arguments {
            ast::FailArguments::Tuple(arguments) => {
                self.check_constructor_arguments(&error_type, arguments, span, &mut errors);
            }
            ast::FailArguments::Struct(fields) => {
                self.check_failure_fields(&error_type, fields, span, &mut errors);
            }
        }

        ExprInfo {
            ty: Type::Never,
            errors,
        }
    }

    fn check_constructor_arguments(
        &mut self,
        error_type: &Type,
        arguments: &[ast::Expr],
        span: Span,
        errors: &mut Vec<ErrorEffect>,
    ) {
        if let Type::Named(name, _) = error_type {
            if let Some(struct_def) = self.resolve_struct(name) {
                let fields = struct_def.fields;
                if fields.len() != arguments.len() {
                    self.error(
                        ARGUMENT_COUNT,
                        format!(
                            "Failure type '{name}' expects {} argument(s), but {} were provided.",
                            fields.len(),
                            arguments.len()
                        ),
                        span,
                        "Match the declared error type fields.",
                    );
                }
                for (field, argument) in fields.iter().zip(arguments) {
                    let info = self.check_expr(argument, Some(&field.ty));
                    append_effects(errors, info.errors);
                }
                return;
            }
            if let Some(enum_def) = self.resolve_enum(name) {
                if enum_def.variants.len() != 1 {
                    self.error(
                        UNDECLARED_ERROR,
                        format!(
                            "Failure type '{name}' has {} variants, so '!{name}(...)' is ambiguous.",
                            enum_def.variants.len()
                        ),
                        span,
                        "Use a dedicated single-variant error enum or a struct error type.",
                    );
                }
                let Some(variant) = enum_def.variants.first() else {
                    self.error(
                        UNDECLARED_ERROR,
                        format!("Failure type '{name}' has no constructible variant."),
                        span,
                        "Declare at least one enum variant.",
                    );
                    return;
                };
                let types = variant
                    .fields
                    .clone()
                    .unwrap_or_default()
                    .into_iter()
                    .map(|field| field.ty)
                    .collect::<Vec<_>>();
                if types.len() != arguments.len() {
                    self.error(
                        ARGUMENT_COUNT,
                        format!(
                            "Failure type '{name}' expects {} argument(s), but {} were provided.",
                            types.len(),
                            arguments.len()
                        ),
                        span,
                        "Match the declared error variant fields.",
                    );
                }
                for (ty, argument) in types.iter().zip(arguments) {
                    let info = self.check_expr(argument, Some(ty));
                    append_effects(errors, info.errors);
                }
                return;
            }
            self.error(
                UNDECLARED_ERROR,
                format!("Failure type '{name}' is not a constructible struct or enum."),
                span,
                "Declare the error type as a struct or enum before constructing it.",
            );
            return;
        }

        for argument in arguments {
            let info = self.check_expr(argument, None);
            append_effects(errors, info.errors);
        }
    }

    fn check_failure_fields(
        &mut self,
        error_type: &Type,
        fields: &[ast::FieldInit],
        span: Span,
        errors: &mut Vec<ErrorEffect>,
    ) {
        let Type::Named(name, _) = error_type else {
            for field in fields {
                let info = self.check_expr(&field.value, None);
                append_effects(errors, info.errors);
            }
            return;
        };
        let Some(struct_def) = self.resolve_struct(name) else {
            if self.resolve_enum(name).is_some() {
                self.error(
                    INVALID_STRUCT_FIELD,
                    format!(
                        "Failure type '{name}' is an enum; use positional construction '!{name}(...)'."
                    ),
                    span,
                    "Construct enum errors with tuple arguments, not named fields.",
                );
            } else {
                self.error(
                    UNDECLARED_ERROR,
                    format!("Failure type '{name}' is not a constructible struct or enum."),
                    span,
                    "Declare the error type as a struct or enum before constructing it.",
                );
            }
            for field in fields {
                let info = self.check_expr(&field.value, None);
                append_effects(errors, info.errors);
            }
            return;
        };

        let mut seen = std::collections::BTreeSet::new();
        for field in fields {
            if !seen.insert(field.name.as_str()) {
                self.error(
                    INVALID_STRUCT_FIELD,
                    format!(
                        "Failure field '{}' is initialized more than once.",
                        field.name
                    ),
                    field.span,
                    "Remove the duplicate initializer.",
                );
            }
            match struct_def
                .fields
                .iter()
                .find(|candidate| candidate.name.as_deref() == Some(field.name.as_str()))
            {
                Some(declared) => {
                    let info = self.check_expr(&field.value, Some(&declared.ty));
                    append_effects(errors, info.errors);
                }
                None => {
                    self.error(
                        INVALID_STRUCT_FIELD,
                        format!("Failure type '{name}' has no field '{}'.", field.name),
                        field.span,
                        "Use a declared field name.",
                    );
                    let info = self.check_expr(&field.value, None);
                    append_effects(errors, info.errors);
                }
            }
        }
        for field in &struct_def.fields {
            if let Some(field_name) = &field.name
                && !fields.iter().any(|candidate| candidate.name == *field_name)
            {
                self.error(
                    INVALID_STRUCT_FIELD,
                    format!("Field '{field_name}' is missing from the failure value."),
                    span,
                    "Initialize every field of the error type.",
                );
            }
        }
    }

    fn literal_type(
        &mut self,
        literal: &ast::Literal,
        expected: Option<&Type>,
        span: Span,
    ) -> Type {
        match literal {
            ast::Literal::Integer(text) => {
                let (raw, suffix_u8) = strip_u8_suffix(text);
                let actual = if suffix_u8 {
                    Type::U8
                } else if expected.is_some_and(Type::is_integer) {
                    expected.cloned().unwrap_or(Type::I32)
                } else {
                    Type::I32
                };

                if let Some(value) = parse_integer(raw)
                    && !integer_fits(value, &actual)
                {
                    self.error(
                        TYPE_MISMATCH,
                        format!("Integer literal '{text}' does not fit in '{actual}'."),
                        span,
                        "Use a wider integer type or a smaller literal.",
                    );
                }
                actual
            }
            ast::Literal::Float(text) => {
                let actual = if expected.is_some_and(|ty| ty.is_float()) {
                    expected.cloned().unwrap_or(Type::F64)
                } else {
                    Type::F64
                };
                let raw = text.replace('_', "");
                if let Ok(value) = raw.parse::<f64>()
                    && actual == Type::F32
                    && (value as f32).is_infinite()
                    && value.is_finite()
                {
                    self.error(
                        TYPE_MISMATCH,
                        format!("Float literal '{text}' does not fit in f32."),
                        span,
                        "Use f64 for larger floating-point values.",
                    );
                }
                actual
            }
            ast::Literal::String(_) => Type::Str,
            ast::Literal::Bytes(_) => Type::Bytes,
            ast::Literal::Bool(_) => Type::Bool,
            ast::Literal::Unit => Type::Unit,
        }
    }

    fn type_mismatch(&mut self, expected: &Type, actual: &Type, span: Span) {
        self.error(
            TYPE_MISMATCH,
            format!("Type mismatch: expected '{expected}', found '{actual}'."),
            span,
            "Convert the value explicitly; Neo Astra has no implicit coercion.",
        );
    }

    fn report_effects(&mut self, errors: &[ErrorEffect]) {
        for error in errors {
            if error.ty.is_opaque() {
                continue;
            }
            self.error(
                UNHANDLED_ERROR,
                format!("Unhandled error effect: {}.", error.ty),
                error.span,
                "Propagate with '!' or handle the error with match.",
            );
        }
    }

    fn record_error(&mut self, error: &ErrorEffect) {
        if error.ty.is_opaque() {
            return;
        }

        let Some(function) = self.current_function.as_ref().cloned() else {
            self.error(
                INVALID_PROPAGATION,
                "Error propagation and failure construction are only valid inside a function.",
                error.span,
                "Move the operation into a fallible function.",
            );
            return;
        };

        if !effect_is_compatible(&function.declared_errors, &error.ty) {
            self.error(
                UNDECLARED_ERROR,
                format!(
                    "Error type '{}' is not declared by function '{}'.",
                    error.ty, function.name
                ),
                error.span,
                format!(
                    "Add '! {}' to the error set of '{}'.",
                    error.ty, function.name
                ),
            );
        }
    }
}

impl Coverage {
    fn missing(&self, shape: &MatchShape) -> Vec<String> {
        let mut missing = Vec::new();
        match shape {
            MatchShape::Option => {
                if !self.some {
                    missing.push("Some(_)".to_owned());
                }
                if !self.none {
                    missing.push("None".to_owned());
                }
            }
            MatchShape::Result => {
                if !self.ok {
                    missing.push("Ok(_)".to_owned());
                }
                if !self.err {
                    missing.push("Err(_)".to_owned());
                }
            }
            MatchShape::Enum(enum_def) => {
                for variant in &enum_def.variants {
                    if !self.variants.contains(&variant.name) {
                        missing.push(variant.name.clone());
                    }
                }
            }
            MatchShape::Bool => {
                if !self.boolean[0] {
                    missing.push("false".to_owned());
                }
                if !self.boolean[1] {
                    missing.push("true".to_owned());
                }
            }
            MatchShape::Tuple => missing.push("a tuple wildcard arm".to_owned()),
            MatchShape::Other => missing.push("a wildcard arm".to_owned()),
        }

        if missing.is_empty() {
            missing.push("a wildcard arm".to_owned());
        }
        missing
    }
}

fn generic_map(type_parameters: &[String]) -> BTreeMap<String, Type> {
    type_parameters
        .iter()
        .map(|parameter| (parameter.clone(), Type::Generic(parameter.clone())))
        .collect()
}

fn substitutions(type_parameters: &[String], arguments: &[Type]) -> BTreeMap<String, Type> {
    type_parameters
        .iter()
        .zip(arguments)
        .map(|(parameter, argument)| (parameter.clone(), argument.clone()))
        .collect()
}

fn check_arity(
    type_parameters: &[String],
    arguments: &[Type],
    name: &str,
    span: Span,
    checker: &mut Checker<'_>,
) -> bool {
    if type_parameters.is_empty() && arguments.is_empty() {
        return true;
    }
    if type_parameters.len() == arguments.len() {
        return true;
    }
    checker.error(
        INVALID_TYPE_ARGUMENT,
        format!(
            "Type '{name}' expects {} type argument(s), but {} were provided.",
            type_parameters.len(),
            arguments.len()
        ),
        span,
        "Match the declared type parameter list.",
    );
    false
}

fn primitive_type(arguments: &[Type], ty: Type, span: Span, checker: &mut Checker<'_>) -> Type {
    if !arguments.is_empty() {
        checker.error(
            INVALID_TYPE_ARGUMENT,
            format!("Primitive type '{ty}' cannot take type arguments."),
            span,
            "Remove the type argument list.",
        );
        return Type::Error;
    }
    ty
}

fn builtin_function(name: &str) -> Option<Type> {
    let signature = match name {
        "print" | "println" | "dbg" => FunctionType {
            type_parameters: Vec::new(),
            parameters: vec![Type::Unknown],
            return_type: Box::new(Type::Unit),
            errors: Vec::new(),
        },
        "assert" => FunctionType {
            type_parameters: Vec::new(),
            parameters: vec![Type::Bool],
            return_type: Box::new(Type::Unit),
            errors: Vec::new(),
        },
        _ => return None,
    };
    Some(Type::Function(signature))
}

fn expected_index_type(ty: &Type) -> Option<Type> {
    match ty {
        Type::Array(_) | Type::Slice(_) | Type::FixedArray(_, _) | Type::Bytes => Some(Type::Usize),
        Type::Map(key, _) => Some((**key).clone()),
        _ => None,
    }
}

fn builtin_constructor_name(callee: &ast::Expr) -> Option<&str> {
    match &callee.kind {
        ast::ExprKind::Identifier(name) if matches!(name.as_str(), "Some" | "Ok" | "Err") => {
            Some(name)
        }
        _ => None,
    }
}

fn channel_create_callee(callee: &ast::Expr) -> bool {
    match &callee.kind {
        ast::ExprKind::Member { target, member } => {
            member == "create"
                && matches!(&target.kind, ast::ExprKind::Identifier(name) if name == "Channel")
        }
        ast::ExprKind::TypeArguments { target, .. } => channel_create_callee(target),
        _ => false,
    }
}

fn builtin_method(ty: &Type, member: &str) -> Option<Type> {
    let signature = match ty {
        Type::Str => match member {
            "is_empty" => FunctionType {
                type_parameters: Vec::new(),
                parameters: Vec::new(),
                return_type: Box::new(Type::Bool),
                errors: Vec::new(),
            },
            "len" => FunctionType {
                type_parameters: Vec::new(),
                parameters: Vec::new(),
                return_type: Box::new(Type::Usize),
                errors: Vec::new(),
            },
            "to_int" => FunctionType {
                type_parameters: Vec::new(),
                parameters: Vec::new(),
                return_type: Box::new(Type::I32),
                errors: vec![Type::Named("ParseIntError".to_owned(), Vec::new())],
            },
            "char_count" => FunctionType {
                type_parameters: Vec::new(),
                parameters: Vec::new(),
                return_type: Box::new(Type::Usize),
                errors: Vec::new(),
            },
            "bytes" => FunctionType {
                type_parameters: Vec::new(),
                parameters: Vec::new(),
                return_type: Box::new(Type::Array(Box::new(Type::U8))),
                errors: Vec::new(),
            },
            "chars" => FunctionType {
                type_parameters: Vec::new(),
                parameters: Vec::new(),
                return_type: Box::new(Type::Array(Box::new(Type::Str))),
                errors: Vec::new(),
            },
            "byte_at" => FunctionType {
                type_parameters: Vec::new(),
                parameters: vec![Type::Usize],
                return_type: Box::new(Type::Option(Box::new(Type::U8))),
                errors: Vec::new(),
            },
            "char_at" => FunctionType {
                type_parameters: Vec::new(),
                parameters: vec![Type::Usize],
                return_type: Box::new(Type::Option(Box::new(Type::Str))),
                errors: Vec::new(),
            },
            "slice" => FunctionType {
                type_parameters: Vec::new(),
                parameters: vec![Type::Usize, Type::Usize],
                return_type: Box::new(Type::Str),
                errors: vec![Type::Named("SliceError".to_owned(), Vec::new())],
            },
            _ => return None,
        },
        Type::Bytes => match member {
            "is_empty" => FunctionType {
                type_parameters: Vec::new(),
                parameters: Vec::new(),
                return_type: Box::new(Type::Bool),
                errors: Vec::new(),
            },
            "len" => FunctionType {
                type_parameters: Vec::new(),
                parameters: Vec::new(),
                return_type: Box::new(Type::Usize),
                errors: Vec::new(),
            },
            "slice" => FunctionType {
                type_parameters: Vec::new(),
                parameters: vec![Type::Usize, Type::Usize],
                return_type: Box::new(Type::Bytes),
                errors: Vec::new(),
            },
            _ => return None,
        },
        Type::Array(element) | Type::Slice(element) | Type::FixedArray(element, _) => {
            match member {
                "len" => FunctionType {
                    type_parameters: Vec::new(),
                    parameters: Vec::new(),
                    return_type: Box::new(Type::Usize),
                    errors: Vec::new(),
                },
                "push" => FunctionType {
                    type_parameters: Vec::new(),
                    parameters: vec![(**element).clone()],
                    return_type: Box::new(Type::Unit),
                    errors: Vec::new(),
                },
                "get" => FunctionType {
                    type_parameters: Vec::new(),
                    parameters: vec![Type::Usize],
                    return_type: Box::new(Type::Option(element.clone())),
                    errors: Vec::new(),
                },
                _ => return None,
            }
        }
        Type::Map(key, value) => match member {
            "len" => FunctionType {
                type_parameters: Vec::new(),
                parameters: Vec::new(),
                return_type: Box::new(Type::Usize),
                errors: Vec::new(),
            },
            "get" => FunctionType {
                type_parameters: Vec::new(),
                parameters: vec![(**key).clone()],
                return_type: Box::new(Type::Option(value.clone())),
                errors: Vec::new(),
            },
            "contains" => FunctionType {
                type_parameters: Vec::new(),
                parameters: vec![(**key).clone()],
                return_type: Box::new(Type::Bool),
                errors: Vec::new(),
            },
            "insert" => FunctionType {
                type_parameters: Vec::new(),
                parameters: vec![(**key).clone(), (**value).clone()],
                return_type: Box::new(Type::Option(value.clone())),
                errors: Vec::new(),
            },
            _ => return None,
        },
        Type::Option(inner) => match member {
            "is_some" | "is_none" => FunctionType {
                type_parameters: Vec::new(),
                parameters: Vec::new(),
                return_type: Box::new(Type::Bool),
                errors: Vec::new(),
            },
            "unwrap" => FunctionType {
                type_parameters: Vec::new(),
                parameters: Vec::new(),
                return_type: inner.clone(),
                errors: Vec::new(),
            },
            "unwrap_or" => FunctionType {
                type_parameters: Vec::new(),
                parameters: vec![(**inner).clone()],
                return_type: inner.clone(),
                errors: Vec::new(),
            },
            _ => return None,
        },
        Type::Result(ok, error) => match member {
            "is_ok" | "is_err" => FunctionType {
                type_parameters: Vec::new(),
                parameters: Vec::new(),
                return_type: Box::new(Type::Bool),
                errors: Vec::new(),
            },
            "unwrap" => FunctionType {
                type_parameters: Vec::new(),
                parameters: Vec::new(),
                return_type: ok.clone(),
                errors: Vec::new(),
            },
            "unwrap_or" => FunctionType {
                type_parameters: Vec::new(),
                parameters: vec![(**ok).clone()],
                return_type: ok.clone(),
                errors: Vec::new(),
            },
            "error" => FunctionType {
                type_parameters: Vec::new(),
                parameters: Vec::new(),
                return_type: error.clone(),
                errors: Vec::new(),
            },
            _ => return None,
        },
        Type::Named(name, arguments) if name == "Task" => match member {
            "join" => FunctionType {
                type_parameters: Vec::new(),
                parameters: Vec::new(),
                return_type: Box::new(arguments.first().cloned().unwrap_or(Type::Unit)),
                errors: Vec::new(),
            },
            "cancel" => FunctionType {
                type_parameters: Vec::new(),
                parameters: Vec::new(),
                return_type: Box::new(Type::Unit),
                errors: Vec::new(),
            },
            "canceled" => FunctionType {
                type_parameters: Vec::new(),
                parameters: Vec::new(),
                return_type: Box::new(Type::Bool),
                errors: Vec::new(),
            },
            _ => return None,
        },
        Type::Named(name, arguments) if name == "Ptr" => {
            let element = arguments.first().cloned().unwrap_or(Type::Unknown);
            match member {
                "read" => FunctionType {
                    type_parameters: Vec::new(),
                    parameters: vec![Type::Usize],
                    return_type: Box::new(element),
                    errors: Vec::new(),
                },
                "write" => FunctionType {
                    type_parameters: Vec::new(),
                    parameters: vec![Type::Usize, element],
                    return_type: Box::new(Type::Unit),
                    errors: Vec::new(),
                },
                _ => return None,
            }
        }
        Type::Named(name, _) if name == "Region" => match member {
            "alloc" => FunctionType {
                type_parameters: vec!["T".to_owned()],
                parameters: vec![Type::Usize],
                return_type: Box::new(Type::Named(
                    "Ptr".to_owned(),
                    vec![Type::Generic("T".to_owned())],
                )),
                errors: Vec::new(),
            },
            "free" => FunctionType {
                type_parameters: Vec::new(),
                parameters: Vec::new(),
                return_type: Box::new(Type::Unit),
                errors: Vec::new(),
            },
            _ => return None,
        },
        Type::Named(name, arguments) if name == "Channel" => {
            let element = arguments.first().cloned().unwrap_or(Type::Unknown);
            match member {
                "send" => FunctionType {
                    type_parameters: Vec::new(),
                    parameters: vec![element],
                    return_type: Box::new(Type::Unit),
                    errors: vec![Type::Named("ChannelError".to_owned(), Vec::new())],
                },
                "receive" => FunctionType {
                    type_parameters: Vec::new(),
                    parameters: Vec::new(),
                    return_type: Box::new(element),
                    errors: vec![Type::Named("ChannelError".to_owned(), Vec::new())],
                },
                "close" => FunctionType {
                    type_parameters: Vec::new(),
                    parameters: Vec::new(),
                    return_type: Box::new(Type::Unit),
                    errors: Vec::new(),
                },
                _ => return None,
            }
        }
        _ if ty.is_integer() => match member {
            "wrapping_add" | "wrapping_sub" | "wrapping_mul" | "wrapping_div" => FunctionType {
                type_parameters: Vec::new(),
                parameters: vec![ty.clone()],
                return_type: Box::new(ty.clone()),
                errors: Vec::new(),
            },
            "checked_add" | "checked_sub" | "checked_mul" | "checked_div" => FunctionType {
                type_parameters: Vec::new(),
                parameters: vec![ty.clone()],
                return_type: Box::new(ty.clone()),
                errors: vec![Type::Named("OverflowError".to_owned(), Vec::new())],
            },
            _ => return None,
        },
        _ => return None,
    };
    Some(Type::Function(signature))
}

fn contains_generic(ty: &Type) -> bool {
    match ty {
        Type::Generic(_) => true,
        Type::Option(inner) | Type::Array(inner) | Type::Slice(inner) => contains_generic(inner),
        Type::FixedArray(inner, _) => contains_generic(inner),
        Type::Result(ok, error) | Type::Map(ok, error) => {
            contains_generic(ok) || contains_generic(error)
        }
        Type::Tuple(elements) => elements.iter().any(contains_generic),
        Type::Function(function) => {
            function.parameters.iter().any(contains_generic)
                || contains_generic(&function.return_type)
                || function.errors.iter().any(contains_generic)
        }
        Type::Named(_, arguments) => arguments.iter().any(contains_generic),
        _ => false,
    }
}

fn infer_substitutions(
    parameter: &Type,
    argument: &Type,
    substitutions: &mut BTreeMap<String, Type>,
) {
    if argument.is_opaque() || argument == &Type::Never {
        return;
    }
    match (parameter, argument) {
        (Type::Generic(name), actual) => {
            substitutions
                .entry(name.clone())
                .or_insert_with(|| actual.clone());
        }
        (Type::Option(parameter), Type::Option(argument)) => {
            infer_substitutions(parameter, argument, substitutions);
        }
        (
            Type::Result(parameter_ok, parameter_error),
            Type::Result(argument_ok, argument_error),
        ) => {
            infer_substitutions(parameter_ok, argument_ok, substitutions);
            infer_substitutions(parameter_error, argument_error, substitutions);
        }
        (Type::Array(parameter), Type::Array(argument))
        | (Type::Array(parameter), Type::Slice(argument))
        | (Type::Slice(parameter), Type::Array(argument))
        | (Type::Slice(parameter), Type::Slice(argument)) => {
            infer_substitutions(parameter, argument, substitutions);
        }
        (Type::FixedArray(parameter, _), Type::FixedArray(argument, _)) => {
            infer_substitutions(parameter, argument, substitutions);
        }
        (Type::Map(parameter_key, parameter_value), Type::Map(argument_key, argument_value)) => {
            infer_substitutions(parameter_key, argument_key, substitutions);
            infer_substitutions(parameter_value, argument_value, substitutions);
        }
        (Type::Tuple(parameters), Type::Tuple(arguments)) => {
            for (parameter, argument) in parameters.iter().zip(arguments) {
                infer_substitutions(parameter, argument, substitutions);
            }
        }
        (Type::Named(parameter_name, parameters), Type::Named(argument_name, arguments))
            if parameter_name == argument_name =>
        {
            for (parameter, argument) in parameters.iter().zip(arguments) {
                infer_substitutions(parameter, argument, substitutions);
            }
        }
        (Type::Function(parameter), Type::Function(argument)) => {
            for (parameter, argument) in parameter.parameters.iter().zip(&argument.parameters) {
                infer_substitutions(parameter, argument, substitutions);
            }
            infer_substitutions(&parameter.return_type, &argument.return_type, substitutions);
        }
        _ => {}
    }
}

fn effect_is_compatible(declared: &[Type], error: &Type) -> bool {
    declared.iter().any(|candidate| candidate == error)
}

fn append_unique(target: &mut Vec<Type>, values: Vec<Type>) {
    for value in values {
        if !target.iter().any(|existing| existing == &value) {
            target.push(value);
        }
    }
}

fn append_effects(target: &mut Vec<ErrorEffect>, values: Vec<ErrorEffect>) {
    for value in values {
        if !target.iter().any(|existing| existing.ty == value.ty) {
            target.push(value);
        }
    }
}

fn merge_effects(left: &[ErrorEffect], right: &[ErrorEffect]) -> Vec<ErrorEffect> {
    let mut merged = left.to_vec();
    append_effects(&mut merged, right.to_vec());
    merged
}

fn block_value_type(info: &BlockInfo) -> Type {
    if info.diverges {
        return Type::Never;
    }
    info.tail
        .as_ref()
        .map(|tail| tail.ty.clone())
        .unwrap_or(Type::Unit)
}

fn block_tail_errors(info: &BlockInfo) -> Vec<ErrorEffect> {
    info.tail
        .as_ref()
        .map(|tail| tail.errors.clone())
        .unwrap_or_default()
}

fn pattern_is_catch_all(pattern: &ast::Pattern, ty: &Type) -> bool {
    match &pattern.kind {
        ast::PatternKind::Wildcard => true,
        ast::PatternKind::Identifier(name) => !(name == "None" && matches!(ty, Type::Option(_))),
        _ => false,
    }
}

fn pattern_row<'a>(
    pattern: &'a ast::Pattern,
    ty: &Type,
    key: &ConstructorKey,
    field_count: usize,
) -> Option<Vec<RowPattern<'a>>> {
    let (pattern_key, arguments) = match &pattern.kind {
        ast::PatternKind::Wildcard => {
            return Some(vec![RowPattern::Wildcard; field_count]);
        }
        ast::PatternKind::Identifier(name) => {
            if name == "None" && matches!(ty, Type::Option(_)) {
                (ConstructorKey::None, Vec::new())
            } else {
                return Some(vec![RowPattern::Wildcard; field_count]);
            }
        }
        ast::PatternKind::Literal(ast::Literal::Bool(value)) => {
            (ConstructorKey::Bool(*value), Vec::new())
        }
        ast::PatternKind::Literal(ast::Literal::Unit) if matches!(ty, Type::Unit) => {
            (ConstructorKey::Unit, Vec::new())
        }
        ast::PatternKind::Literal(_) => return None,
        ast::PatternKind::Tuple(patterns) if matches!(ty, Type::Tuple(_)) => (
            ConstructorKey::Tuple,
            patterns.iter().map(RowPattern::Pattern).collect(),
        ),
        ast::PatternKind::Tuple(_) => return None,
        ast::PatternKind::Constructor { path, arguments } => {
            let name = path.segments.last().map(String::as_str).unwrap_or_default();
            match (name, ty) {
                ("Some", Type::Option(_)) => (
                    ConstructorKey::Some,
                    arguments.iter().map(RowPattern::Pattern).collect(),
                ),
                ("Ok", Type::Result(_, _)) => (
                    ConstructorKey::Ok,
                    arguments.iter().map(RowPattern::Pattern).collect(),
                ),
                ("Err", Type::Result(_, _)) => (
                    ConstructorKey::Err,
                    arguments.iter().map(RowPattern::Pattern).collect(),
                ),
                _ => return None,
            }
        }
        ast::PatternKind::Variant {
            path,
            variant,
            arguments,
        } => {
            let enum_name = path.segments.last().map(String::as_str).unwrap_or_default();
            let nested = arguments
                .as_deref()
                .unwrap_or(&[])
                .iter()
                .map(RowPattern::Pattern)
                .collect::<Vec<_>>();
            match enum_name {
                "Option" if variant == "Some" => (ConstructorKey::Some, nested),
                "Option" if variant == "None" => (ConstructorKey::None, Vec::new()),
                "Result" if variant == "Ok" => (ConstructorKey::Ok, nested),
                "Result" if variant == "Err" => (ConstructorKey::Err, nested),
                _ => (ConstructorKey::Variant(variant.clone()), nested),
            }
        }
    };

    if &pattern_key != key {
        return None;
    }

    let mut row = arguments;
    row.truncate(field_count);
    while row.len() < field_count {
        row.push(RowPattern::Wildcard);
    }
    Some(row)
}

fn result_parts(ty: &Type, effects: &[Type]) -> (Type, Type) {
    if let Type::Result(ok, error) = ty {
        return ((**ok).clone(), (**error).clone());
    }
    if !effects.is_empty() {
        return (
            ty.clone(),
            if effects.len() == 1 {
                effects[0].clone()
            } else {
                Type::Unknown
            },
        );
    }
    (ty.clone(), Type::Unknown)
}

fn annotation_name(expression: &ast::Expr) -> Option<(String, Span)> {
    match &expression.kind {
        ast::ExprKind::Identifier(name) => Some((name.clone(), expression.span)),
        ast::ExprKind::Literal(ast::Literal::String(name)) => Some((name.clone(), expression.span)),
        _ => None,
    }
}

fn is_numeric_literal(expression: &ast::Expr) -> bool {
    matches!(
        expression.kind,
        ast::ExprKind::Literal(ast::Literal::Integer(_) | ast::Literal::Float(_))
    )
}

fn integer_literal_value(expression: &ast::Expr) -> Option<usize> {
    let ast::ExprKind::Literal(ast::Literal::Integer(text)) = &expression.kind else {
        return None;
    };
    let (raw, _) = strip_u8_suffix(text);
    usize::try_from(parse_integer(raw)?).ok()
}

fn strip_u8_suffix(text: &str) -> (&str, bool) {
    if let Some(raw) = text.strip_suffix("u8") {
        (raw, true)
    } else {
        (text, false)
    }
}

fn parse_integer(text: &str) -> Option<u128> {
    let cleaned = text.replace('_', "");
    if let Some(hex) = cleaned
        .strip_prefix("0x")
        .or_else(|| cleaned.strip_prefix("0X"))
    {
        u128::from_str_radix(hex, 16).ok()
    } else if let Some(binary) = cleaned
        .strip_prefix("0b")
        .or_else(|| cleaned.strip_prefix("0B"))
    {
        u128::from_str_radix(binary, 2).ok()
    } else {
        cleaned.parse::<u128>().ok()
    }
}

fn integer_fits(value: u128, ty: &Type) -> bool {
    match ty {
        Type::I8 => value <= i8::MAX as u128,
        Type::I16 => value <= i16::MAX as u128,
        Type::I32 => value <= i32::MAX as u128,
        Type::I64 => value <= i64::MAX as u128,
        Type::Isize => value <= i64::MAX as u128,
        Type::U8 => value <= u8::MAX as u128,
        Type::U16 => value <= u16::MAX as u128,
        Type::U32 => value <= u32::MAX as u128,
        Type::U64 => value <= u64::MAX as u128,
        Type::Usize => value <= u64::MAX as u128,
        _ => true,
    }
}

fn expr_has_call(expression: &ast::Expr) -> bool {
    match &expression.kind {
        ast::ExprKind::Call { .. } => true,
        ast::ExprKind::Unary { operand, .. } => expr_has_call(operand),
        ast::ExprKind::Binary { left, right, .. } => expr_has_call(left) || expr_has_call(right),
        ast::ExprKind::Assign { target, value } => expr_has_call(target) || expr_has_call(value),
        ast::ExprKind::Index { target, index } => expr_has_call(target) || expr_has_call(index),
        ast::ExprKind::Member { target, .. } => expr_has_call(target),
        ast::ExprKind::TypeArguments { target, .. } => expr_has_call(target),
        ast::ExprKind::Propagate(inner) => expr_has_call(inner),
        ast::ExprKind::Array(elements) | ast::ExprKind::Tuple(elements) => {
            elements.iter().any(expr_has_call)
        }
        ast::ExprKind::Map(entries) => entries
            .iter()
            .any(|entry| expr_has_call(&entry.key) || expr_has_call(&entry.value)),
        ast::ExprKind::StructInit { fields, .. } => {
            fields.iter().any(|field| expr_has_call(&field.value))
        }
        ast::ExprKind::Block(block) | ast::ExprKind::Spawn { body: block } => {
            block_has_calls(block)
        }
        ast::ExprKind::If {
            condition,
            then_branch,
            else_branch,
        } => {
            expr_has_call(condition)
                || block_has_calls(then_branch)
                || match else_branch {
                    Some(ast::ElseBranch::Block(block)) => block_has_calls(block),
                    Some(ast::ElseBranch::If(inner)) => expr_has_call(inner),
                    None => false,
                }
        }
        ast::ExprKind::Match { value, arms } => {
            expr_has_call(value)
                || arms.iter().any(|arm| match &arm.body {
                    ast::MatchArmBody::Expr(inner) => expr_has_call(inner),
                    ast::MatchArmBody::Block(block) => block_has_calls(block),
                })
        }
        ast::ExprKind::Fail { arguments, .. } => match arguments {
            ast::FailArguments::Tuple(values) => values.iter().any(expr_has_call),
            ast::FailArguments::Struct(fields) => {
                fields.iter().any(|field| expr_has_call(&field.value))
            }
        },
        ast::ExprKind::Panic { message } | ast::ExprKind::Abort { message } => {
            expr_has_call(message)
        }
        ast::ExprKind::Literal(_) | ast::ExprKind::Identifier(_) => false,
    }
}

fn block_has_calls(block: &ast::Block) -> bool {
    block.statements.iter().any(stmt_has_calls)
}

fn is_valid_c_identifier(symbol: &str) -> bool {
    let mut characters = symbol.chars();
    match characters.next() {
        Some(first) if first == '_' || first.is_ascii_alphabetic() => {}
        _ => return false,
    }
    characters.all(|character| character == '_' || character.is_ascii_alphanumeric())
}

fn stmt_has_calls(statement: &ast::Stmt) -> bool {
    match &statement.kind {
        ast::StmtKind::Let { value, .. } => expr_has_call(value),
        ast::StmtKind::Return { value } => value.as_ref().is_some_and(expr_has_call),
        ast::StmtKind::Break | ast::StmtKind::Continue => false,
        ast::StmtKind::Defer(inner) => stmt_has_calls(inner),
        ast::StmtKind::While { condition, body } => {
            expr_has_call(condition) || block_has_calls(body)
        }
        ast::StmtKind::For { iterable, body, .. } => {
            expr_has_call(iterable) || block_has_calls(body)
        }
        ast::StmtKind::Expr(inner) => expr_has_call(inner),
        ast::StmtKind::Scoped(scoped) => scoped.statements.iter().any(stmt_has_calls),
    }
}
