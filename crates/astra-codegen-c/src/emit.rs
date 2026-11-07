use std::collections::{BTreeMap, HashMap, HashSet, VecDeque};
use std::fmt::Write as _;

use astra_core::ast::{
    self, BinaryOp, ElseBranch, Expr, ExprKind, FailArguments, Literal, MatchArm, MatchArmBody,
    Pattern, PatternKind, Stmt, StmtKind, UnaryOp,
};
use astra_core::hir::ImportKind;
use astra_core::types::Type;

use crate::runtime::RUNTIME_HEADER;
use crate::ty::{Annotations, Reject, TyCtxt, TypeEnv, path_to_type, sanitize, to_type};
use crate::{BackendError, CModule, GeneratedC, IntegerPolicy};

const C_KEYWORDS: &[&str] = &[
    "auto",
    "break",
    "case",
    "char",
    "const",
    "continue",
    "default",
    "do",
    "double",
    "else",
    "enum",
    "extern",
    "float",
    "for",
    "goto",
    "if",
    "inline",
    "int",
    "long",
    "register",
    "restrict",
    "return",
    "short",
    "signed",
    "size_t",
    "static",
    "struct",
    "switch",
    "typedef",
    "union",
    "unsigned",
    "void",
    "volatile",
    "while",
    "_Bool",
    "_Complex",
    "_Imaginary",
    "_Alignas",
    "_Alignof",
    "_Atomic",
    "_Generic",
    "_Noreturn",
    "_Static_assert",
    "_Thread_local",
    "bool",
    "true",
    "false",
];

fn is_c_keyword(name: &str) -> bool {
    C_KEYWORDS.contains(&name)
}

fn field_c(name: &str) -> String {
    if is_c_keyword(name) {
        format!("f_{name}")
    } else {
        name.to_string()
    }
}

fn reject(code: &'static str, message: impl Into<String>) -> Reject {
    Reject {
        code,
        message: message.into(),
    }
}

pub fn emit(module: &CModule) -> Result<GeneratedC, BackendError> {
    if module
        .ast
        .items
        .iter()
        .any(|item| matches!(item.kind, ast::ItemKind::Import(_)))
    {
        return Err(BackendError::new(
            "NS8019",
            "imports are not supported by the C backend first slice; it emits a single self-contained module",
        ));
    }

    let mut scripts = Vec::new();
    collect_scripts(&module.ast.items, &mut scripts);
    for s in &scripts {
        if s.language != "c" && s.language != "cpp" {
            return Err(BackendError::new(
                "NS7002",
                format!(
                    "Script language '{}' is not supported by the active target 'c'.",
                    s.language
                ),
            ));
        }
    }

    let env = TypeEnv::collect(&module.ast);
    let mut tcx = TyCtxt::new(env);
    prescan(&mut tcx, module);
    if tcx.rejected() {
        let r = tcx.rejects.into_iter().next().expect("non-empty");
        return Err(BackendError::new(r.code, &r.message));
    }

    let ann = Annotations::new(&module.annotations);
    let mut e = Emitter {
        module,
        tcx: &mut tcx,
        ann,
        policy: module.integer_policy,
        scripts: scripts.clone(),
        linked: false,
        prefix: String::new(),
        imports: HashMap::new(),
        namespaces: HashMap::new(),
        default_imports: HashSet::new(),
        dep_headers: Vec::new(),
        emit_main_entry: true,
        library: false,
        pin_returns: false,
        out: String::new(),
        pre: String::new(),
        fresh: 0,
        cur_ret: Type::Unit,
        cur_err: None,
        expected: None,
        instantiations: BTreeMap::new(),
        pending_instantiations: VecDeque::new(),
        emitted_instantiations: HashSet::new(),
        defers: Vec::new(),
        roots: None,
        hoist: true,
        scope_tasks: Vec::new(),
        pending_pattern_locals: Vec::new(),
        pending_pattern_slots: Vec::new(),
        task_defs: String::new(),
        task_protos: Vec::new(),
    };
    e.emit_program();

    if e.tcx.rejected() {
        let r = e.tcx.rejects.clone().into_iter().next().expect("non-empty");
        return Err(BackendError::new(r.code, &r.message));
    }
    let script_sources = e.emit_scripts();
    Ok(GeneratedC {
        source: e.out,
        header: None,
        host_shim: crate::runtime::host_shim(),
        script_sources,
        target: "c".to_string(),
    })
}

pub fn emit_project(
    entries: &[crate::ProjectEntry],
    emit_main: bool,
) -> Result<Vec<GeneratedC>, BackendError> {
    use std::collections::HashMap;

    let header_map: HashMap<String, String> = entries
        .iter()
        .map(|e| {
            let stem = module_key_stem(&e.key);
            (e.key.clone(), format!("{stem}.h"))
        })
        .collect();

    let mut results = Vec::new();
    let last_key = entries.last().map(|e| e.key.clone());

    let envs_by_key: std::collections::BTreeMap<String, TypeEnv> = entries
        .iter()
        .map(|e| (e.key.clone(), TypeEnv::collect(&e.module.ast)))
        .collect();
    for entry in entries {
        let mut scripts = Vec::new();
        collect_scripts(&entry.module.ast.items, &mut scripts);
        for s in &scripts {
            if s.language != "c" && s.language != "cpp" {
                return Err(BackendError::new(
                    "NS7002",
                    format!(
                        "module '{}': script language '{}' is not supported by target 'c'",
                        entry.module.name, s.language
                    ),
                ));
            }
        }

        let stem = module_key_stem(&entry.key);
        let env = TypeEnv::collect(&entry.module.ast);
        let mut tcx = TyCtxt::new(env);

        tcx.set_type_prefix(&stem);

        let (imports, namespaces, default_imports) = collect_import_tables(&entry.module.hir);

        install_foreign_types(&mut tcx, &imports, &namespaces, &envs_by_key);

        prescan(&mut tcx, &entry.module);
        if tcx.rejected() {
            let r = tcx.rejects.into_iter().next().expect("non-empty");
            return Err(BackendError::new(r.code, &r.message));
        }

        let dep_headers: Vec<String> = entry
            .dep_keys
            .iter()
            .filter_map(|k| header_map.get(k).cloned())
            .collect();

        let ann = Annotations::new(&entry.module.annotations);
        let mut e = Emitter {
            module: &entry.module,
            tcx: &mut tcx,
            ann,
            policy: entry.module.integer_policy,
            scripts: scripts.clone(),
            linked: true,
            prefix: format!("{stem}_"),
            imports,
            namespaces,
            default_imports,
            dep_headers: dep_headers.clone(),

            emit_main_entry: emit_main && last_key.as_deref() == Some(entry.key.as_str()),
            library: !emit_main,
            pin_returns: false,
            out: String::new(),
            pre: String::new(),
            fresh: 0,
            cur_ret: Type::Unit,
            cur_err: None,
            expected: None,
            instantiations: BTreeMap::new(),
            pending_instantiations: VecDeque::new(),
            emitted_instantiations: HashSet::new(),
            defers: Vec::new(),
            roots: None,
            hoist: true,
            scope_tasks: Vec::new(),
            pending_pattern_locals: Vec::new(),
            pending_pattern_slots: Vec::new(),
            task_defs: String::new(),
            task_protos: Vec::new(),
        };
        e.emit_program_linked(&dep_headers);
        if e.tcx.rejected() {
            let r = e.tcx.rejects.clone().into_iter().next().expect("non-empty");
            return Err(BackendError::new(r.code, &r.message));
        }
        let script_sources = e.emit_scripts();
        let source = e.out.clone();
        let header = e.emit_header();

        results.push(GeneratedC {
            source,
            header: Some(header),
            host_shim: crate::runtime::host_shim(),
            script_sources,
            target: "c".to_string(),
        });
    }
    Ok(results)
}

type ImportTables = (
    HashMap<String, (String, String)>,
    HashMap<String, String>,
    HashSet<String>,
);

fn collect_import_tables(hir: &astra_core::hir::Module) -> ImportTables {
    let mut named = HashMap::new();
    let mut namespaces = HashMap::new();
    let mut default_imports = HashSet::new();
    for import in &hir.imports {
        match &import.kind {
            ImportKind::Named { bindings } => {
                for binding in bindings {
                    named.insert(
                        binding.local.clone(),
                        (import.module.clone(), binding.imported.clone()),
                    );
                }
            }
            ImportKind::Namespace { alias } => {
                namespaces.insert(alias.clone(), import.module.clone());
            }
            ImportKind::Default { local } => {
                default_imports.insert(local.clone());
            }
            ImportKind::DefaultNamed { default, bindings } => {
                default_imports.insert(default.clone());
                for binding in bindings {
                    named.insert(
                        binding.local.clone(),
                        (import.module.clone(), binding.imported.clone()),
                    );
                }
            }
            ImportKind::SideEffect => {}
        }
    }
    (named, namespaces, default_imports)
}

fn module_key_stem(key: &str) -> String {
    key.strip_suffix(".ns")
        .unwrap_or(key)
        .replace(['/', '.', '-'], "_")
}

fn install_foreign_types(
    tcx: &mut TyCtxt,
    imports: &HashMap<String, (String, String)>,
    namespaces: &HashMap<String, String>,
    envs: &BTreeMap<String, TypeEnv>,
) {
    let mut table: BTreeMap<String, crate::ty::ForeignDecl> = BTreeMap::new();

    let mut named: Vec<_> = imports.iter().collect();
    named.sort();
    for (written, (dep_key, imported)) in named {
        let Some(env) = envs.get(dep_key) else {
            continue;
        };
        let Some(shape) = env.shape(imported).cloned() else {
            continue;
        };
        let decl = crate::ty::ForeignDecl {
            stem: module_key_stem(dep_key),
            local: imported.clone(),
            shape,
        };
        table.insert(written.clone(), decl.clone());
        table.insert(imported.clone(), decl);
    }

    let mut namespace_list: Vec<_> = namespaces.iter().collect();
    namespace_list.sort();
    for (alias, dep_key) in namespace_list {
        let Some(env) = envs.get(dep_key) else {
            continue;
        };
        let stem = module_key_stem(dep_key);
        for (name, shape) in &env.decls {
            let decl = crate::ty::ForeignDecl {
                stem: stem.clone(),
                local: name.clone(),
                shape: shape.clone(),
            };
            table.insert(format!("{alias}.{name}"), decl.clone());
            table.entry(name.clone()).or_insert(decl);
        }
    }

    for (dep_key, env) in envs {
        let stem = module_key_stem(dep_key);
        for (name, shape) in &env.decls {
            table.entry(name.clone()).or_insert(crate::ty::ForeignDecl {
                stem: stem.clone(),
                local: name.clone(),
                shape: shape.clone(),
            });
        }
    }
    for (name, decl) in table {
        tcx.add_foreign(name, decl);
    }
}

fn prescan(tcx: &mut TyCtxt, module: &CModule) {
    for item in &module.ast.items {
        match &item.kind {
            ast::ItemKind::Function(f) => {
                prescan_fn(tcx, f);
            }
            ast::ItemKind::Struct(s) => {
                let shape = crate::ty::DeclShape::Struct(crate::ty::StructShape {
                    tuple: matches!(s.fields, ast::StructFields::Tuple(_)),
                    fields: match &s.fields {
                        ast::StructFields::Named(fs) => fs
                            .iter()
                            .map(|f| (f.name.clone(), to_type(&f.ty)))
                            .collect(),
                        ast::StructFields::Tuple(ts) => ts
                            .iter()
                            .enumerate()
                            .map(|(i, t)| (format!("_{i}"), to_type(t)))
                            .collect(),
                    },
                });
                let _ = tcx.c_type(&Type::Named(s.name.clone(), Vec::new()));

                tcx.env.decls.insert(s.name.clone(), shape);
            }
            ast::ItemKind::Enum(e) => {
                let _ = tcx.c_type(&Type::Named(e.name.clone(), Vec::new()));
            }
            ast::ItemKind::Constant(c) => {
                let _ = tcx.c_type(&to_type(&c.ty));
            }
            ast::ItemKind::ScopedItems(s) => {
                prescan_items(tcx, &s.items);
            }
            ast::ItemKind::ScriptBlock(s) => {
                prescan_script(tcx, s);
            }
            ast::ItemKind::Export(exp) => {
                if let ast::ExportForm::DefaultFunction(f) = &exp.form {
                    prescan_fn(tcx, f);
                }
            }
            _ => {}
        }
    }
    for ann in &module.annotations {
        if !mentions_unresolved(&ann.ty) {
            let _ = tcx.c_type(&ann.ty);
        }
    }
}

fn mentions_unresolved(ty: &Type) -> bool {
    match ty {
        Type::Unknown | Type::Error | Type::Never | Type::Function(_) | Type::Generic(_) => true,
        Type::Option(inner) | Type::Array(inner) | Type::Slice(inner) => mentions_unresolved(inner),
        Type::FixedArray(inner, _) => mentions_unresolved(inner),
        Type::Result(ok, err) => mentions_unresolved(ok) || mentions_unresolved(err),
        Type::Map(key, value) => mentions_unresolved(key) || mentions_unresolved(value),
        Type::Tuple(items) => items.iter().any(mentions_unresolved),
        Type::Named(_, arguments) => arguments.iter().any(mentions_unresolved),
        _ => false,
    }
}

fn prescan_items(tcx: &mut TyCtxt, items: &[ast::Item]) {
    for item in items {
        match &item.kind {
            ast::ItemKind::Function(f) => prescan_fn(tcx, f),
            ast::ItemKind::Constant(c) => {
                let _ = tcx.c_type(&to_type(&c.ty));
            }
            ast::ItemKind::ScriptBlock(s) => prescan_script(tcx, s),
            ast::ItemKind::ScopedItems(s) => prescan_items(tcx, &s.items),
            _ => {}
        }
    }
}

fn prescan_script(tcx: &mut TyCtxt, s: &ast::ScriptBlock) {
    for exp in &s.exports {
        for param in &exp.parameters {
            let _ = tcx.c_type(&to_type(&param.ty));
        }
        if let Some(ret) = &exp.return_type {
            let _ = tcx.c_type(&to_type(ret));
        }
        for err in &exp.error_set {
            let _ = tcx.c_type(&path_to_type(err));
        }
    }
}

fn collect_scripts(items: &[ast::Item], scripts: &mut Vec<ast::ScriptBlock>) {
    for item in items {
        match &item.kind {
            ast::ItemKind::ScriptBlock(s) => scripts.push(s.clone()),
            ast::ItemKind::ScopedItems(s) => collect_scripts(&s.items, scripts),
            _ => {}
        }
    }
}

fn prescan_fn(tcx: &mut TyCtxt, f: &ast::FunctionDecl) {
    if !f.type_parameters.is_empty() {
        return;
    }
    for param in &f.parameters {
        let _ = tcx.c_type(&to_type(&param.ty));
    }
    let ret = f.return_type.as_ref().map(to_type).unwrap_or(Type::Unit);
    let errs: Vec<Type> = f.error_set.iter().map(path_to_type).collect();

    if ret != Type::Unit {
        let _ = tcx.c_type(&ret);
    }

    for err in &errs {
        let _ = tcx.c_type(err);
    }

    if let Some(err) = errs.into_iter().next() {
        let _ = tcx.c_type(&Type::Result(Box::new(ret), Box::new(err)));
    }
}

struct Emitter<'a> {
    module: &'a CModule,
    tcx: &'a mut TyCtxt,
    ann: Annotations,
    policy: IntegerPolicy,
    scripts: Vec<ast::ScriptBlock>,

    linked: bool,

    prefix: String,

    imports: HashMap<String, (String, String)>,

    namespaces: HashMap<String, String>,

    default_imports: HashSet<String>,

    dep_headers: Vec<String>,

    emit_main_entry: bool,

    library: bool,

    pin_returns: bool,
    out: String,
    pre: String,
    fresh: usize,
    cur_ret: Type,
    cur_err: Option<Type>,

    expected: Option<Type>,

    instantiations: BTreeMap<String, Instantiation>,

    pending_instantiations: VecDeque<String>,

    emitted_instantiations: HashSet<String>,
    defers: Vec<Vec<ast::Stmt>>,

    roots: Option<RootFrame>,

    hoist: bool,

    scope_tasks: Vec<Vec<PendingTask>>,

    pending_pattern_locals: Vec<(String, Type)>,

    pending_pattern_slots: Vec<(String, Type, usize)>,

    task_defs: String,
    task_protos: Vec<String>,
}

#[derive(Clone)]
struct PendingTask {
    var: String,
    task_ty: Type,
}

fn task_value_type(task_ty: &Type) -> Type {
    match task_ty {
        Type::Named(name, arguments) if name == "Task" => {
            arguments.first().cloned().unwrap_or(Type::Unit)
        }
        _ => Type::Unit,
    }
}

struct RootFrame {
    var: String,
    used: usize,
    locals: Vec<(String, Type, usize)>,

    all_locals: Vec<(String, Type)>,

    capture_scopes: Vec<(usize, usize)>,
}

#[derive(Clone)]
struct Instantiation {
    decl: ast::FunctionDecl,
    subst: BTreeMap<String, Type>,
}

impl<'a> Emitter<'a> {
    fn fn_symbol(&self, name: &str) -> String {
        format!("ns_{}{}", self.prefix, sanitize(name))
    }

    fn generic_fn(&self, name: &str) -> Option<ast::FunctionDecl> {
        fn find(items: &[ast::Item], name: &str) -> Option<ast::FunctionDecl> {
            for item in items {
                match &item.kind {
                    ast::ItemKind::Function(f)
                        if f.name == name && !f.type_parameters.is_empty() =>
                    {
                        return Some(f.clone());
                    }
                    ast::ItemKind::ScopedItems(s) => {
                        if let Some(found) = find(&s.items, name) {
                            return Some(found);
                        }
                    }
                    _ => {}
                }
            }
            None
        }
        find(&self.module.ast.items, name)
    }

    fn infer_subst(
        &self,
        decl: &ast::FunctionDecl,
        args: &[Expr],
    ) -> Option<BTreeMap<String, Type>> {
        let mut subst = BTreeMap::new();
        for (parameter, argument) in decl.parameters.iter().zip(args) {
            let pattern = to_type(&parameter.ty);
            let concrete = self.tcx.substitute(self.type_of(argument)?);
            bind_type_params(&pattern, &concrete, &decl.type_parameters, &mut subst)?;
        }
        if decl
            .type_parameters
            .iter()
            .any(|parameter| !subst.contains_key(parameter))
        {
            return None;
        }
        Some(subst)
    }

    fn instantiation_symbol(
        &self,
        decl: &ast::FunctionDecl,
        subst: &BTreeMap<String, Type>,
    ) -> String {
        let mut symbol = self.fn_symbol(&decl.name);
        for parameter in &decl.type_parameters {
            symbol.push('_');
            symbol.push_str(&self.tcx.ident(&subst[parameter]));
        }
        symbol
    }

    fn instantiate_call(
        &mut self,
        decl: &ast::FunctionDecl,
        args: &[Expr],
    ) -> Option<(String, String)> {
        let Some(subst) = self.infer_subst(decl, args) else {
            self.tcx.rejects.push(reject(
                "NS8018",
                format!(
                    "cannot infer type arguments for generic function '{}'",
                    decl.name
                ),
            ));
            return None;
        };
        let symbol = self.instantiation_symbol(decl, &subst);
        if !self.instantiations.contains_key(&symbol) {
            self.instantiations.insert(
                symbol.clone(),
                Instantiation {
                    decl: decl.clone(),
                    subst,
                },
            );
            self.pending_instantiations.push_back(symbol.clone());
        }
        let arguments = self.args_c(args);
        Some((symbol, arguments))
    }

    fn emit_instantiations(&mut self) {
        while let Some(symbol) = self.pending_instantiations.pop_front() {
            if !self.emitted_instantiations.insert(symbol.clone()) {
                continue;
            }
            let Some(instantiation) = self.instantiations.get(&symbol).cloned() else {
                continue;
            };
            self.tcx.set_subst(instantiation.subst);
            self.emit_function_named(&instantiation.decl, &symbol);
            self.tcx.clear_subst();
            self.out.push('\n');
        }
    }

    fn emit_instantiation_protos(&mut self) {
        let instantiations = self.instantiations.clone();
        for (symbol, instantiation) in instantiations {
            self.tcx.set_subst(instantiation.subst);
            self.emit_fn_proto_named(&instantiation.decl, &symbol);
            self.tcx.clear_subst();
        }
    }

    fn register_exported_instantiations(&mut self) {
        let items = self.module.ast.items.clone();
        self.register_exported_items(&items);
    }

    fn register_exported_items(&mut self, items: &[ast::Item]) {
        for item in items {
            match &item.kind {
                ast::ItemKind::Function(decl) if !decl.type_parameters.is_empty() => {
                    for attribute in &item.attributes {
                        let Some(spec) = &attribute.export_as else {
                            continue;
                        };
                        let subst: BTreeMap<String, Type> = spec
                            .bindings
                            .iter()
                            .map(|(parameter, ty)| (parameter.clone(), to_type(ty)))
                            .collect();
                        let symbol = spec.symbol.clone();
                        if symbol.is_empty() || self.instantiations.contains_key(&symbol) {
                            continue;
                        }
                        self.instantiations.insert(
                            symbol.clone(),
                            Instantiation {
                                decl: decl.clone(),
                                subst,
                            },
                        );
                        self.pending_instantiations.push_back(symbol);
                    }
                }
                ast::ItemKind::ScopedItems(scoped) => self.register_exported_items(&scoped.items),
                _ => {}
            }
        }
    }

    fn emit_program(&mut self) {
        self.register_exported_instantiations();

        let (fns, consts) = self.collect_decls();

        for c in &consts {
            self.emit_constant(c);
        }
        self.out.push('\n');
        for f in &fns {
            self.emit_function(f);
            self.out.push('\n');
        }
        if let Some(main) = self.find_main() {
            self.emit_main(&main);
        }

        self.emit_instantiations();
        let lifted = std::mem::take(&mut self.task_defs);
        self.out.push_str(&lifted);

        let code = std::mem::take(&mut self.out);

        self.out.push_str(RUNTIME_HEADER);
        self.out.push('\n');
        self.out.push('\n');
        if !self.tcx.typedefs.is_empty() {
            for decl in &self.tcx.typedefs.clone() {
                self.out.push_str(decl);
                self.out.push('\n');
            }
            self.out.push('\n');
        }
        if !self.tcx.shapes.is_empty() {
            for shape in &self.tcx.shapes.clone() {
                self.out.push_str(shape);
                self.out.push('\n');
            }
            self.out.push('\n');
        }

        for f in &fns {
            self.emit_fn_proto(f);
        }
        self.emit_instantiation_protos();
        for proto in self.task_protos.clone() {
            self.out.push_str(&proto);
            self.out.push('\n');
        }
        self.out.push('\n');
        if !self.scripts.is_empty() {
            for s in &self.scripts.clone() {
                for exp in &s.exports {
                    let (ret_c, params_c) = self.script_export_signature(exp);
                    let _ = writeln!(self.out, "{ret_c} {}({params_c});", sanitize(&exp.name));
                }
            }
            self.out.push('\n');
        }
        self.out.push_str(&code);
    }

    fn emit_program_linked(&mut self, dep_headers: &[String]) {
        self.register_exported_instantiations();

        let (fns, consts) = self.collect_decls();

        for c in &consts {
            self.emit_constant(c);
        }
        self.out.push('\n');
        for f in &fns {
            self.emit_function(f);
            self.out.push('\n');
        }
        if self.emit_main_entry
            && let Some(main) = self.find_main()
        {
            self.emit_main(&main);
        }

        self.emit_instantiations();
        let lifted = std::mem::take(&mut self.task_defs);
        self.out.push_str(&lifted);

        let code = std::mem::take(&mut self.out);

        self.out.push_str(RUNTIME_HEADER);
        self.out.push('\n');
        self.out.push('\n');

        for h in dep_headers {
            let _ = writeln!(self.out, "#include \"{h}\"");
        }
        if !dep_headers.is_empty() {
            self.out.push('\n');
        }
        if !self.tcx.typedefs.is_empty() {
            for decl in &self.tcx.typedefs.clone() {
                self.out.push_str(decl);
                self.out.push('\n');
            }
            self.out.push('\n');
        }
        if !self.tcx.shapes.is_empty() {
            for shape in &self.tcx.shapes.clone() {
                self.out.push_str(shape);
                self.out.push('\n');
            }
            self.out.push('\n');
        }

        for f in &fns {
            self.emit_fn_proto(f);
        }
        self.emit_instantiation_protos();
        for proto in self.task_protos.clone() {
            self.out.push_str(&proto);
            self.out.push('\n');
        }
        self.out.push('\n');
        if !self.scripts.is_empty() {
            for s in &self.scripts.clone() {
                for exp in &s.exports {
                    let (ret_c, params_c) = self.script_export_signature(exp);
                    let _ = writeln!(self.out, "{ret_c} {}({params_c});", sanitize(&exp.name));
                }
            }
            self.out.push('\n');
        }
        self.out.push_str(&code);
    }

    fn emit_header(&mut self) -> String {
        let mut h = String::new();
        let guard = format!(
            "NS_MOD_{}_H",
            self.module
                .name
                .replace(['.', '-', '/'], "_")
                .to_uppercase()
        );
        let _ = writeln!(h, "#ifndef {guard}");
        let _ = writeln!(h, "#define {guard}");
        h.push('\n');
        h.push_str(RUNTIME_HEADER);
        h.push('\n');

        for dep in &self.dep_headers {
            let _ = writeln!(h, "#include \"{dep}\"");
        }
        if !self.dep_headers.is_empty() {
            h.push('\n');
        }

        for decl in &self.tcx.typedefs.clone() {
            h.push_str(decl);
            h.push('\n');
        }
        h.push('\n');

        let (fns, _) = self.collect_decls();
        let exported_fns: Vec<_> = fns
            .iter()
            .filter(|f| {
                self.module
                    .hir
                    .items
                    .iter()
                    .any(|item| item.name == f.name && item.exported)
            })
            .collect();
        if !exported_fns.is_empty() {
            h.push_str("#define NS_ASTRA_EXPORTED_PROTOTYPES 1\n");
            for f in exported_fns {
                if f.type_parameters.is_empty() {
                    let (ret_c, _) = self.fn_return_c(f);
                    let params = self.fn_params_c(f);
                    let _ = writeln!(h, "{ret_c} {}({params});", self.fn_symbol(&f.name));
                }
            }
        }

        let mut exported_instances: Vec<String> = Vec::new();
        for item in &self.module.ast.items.clone() {
            if let ast::ItemKind::Function(_) = &item.kind {
                for attribute in &item.attributes {
                    if let Some(spec) = &attribute.export_as {
                        exported_instances.push(spec.symbol.clone());
                    }
                }
            }
        }
        if !exported_instances.is_empty() {
            for symbol in exported_instances {
                let Some(instantiation) = self.instantiations.get(&symbol).cloned() else {
                    continue;
                };
                self.tcx.set_subst(instantiation.subst);
                let (ret_c, _) = self.fn_return_c(&instantiation.decl);
                let params = self.fn_params_c(&instantiation.decl);
                self.tcx.clear_subst();
                let _ = writeln!(h, "{ret_c} {symbol}({params});");
            }
        }
        h.push('\n');
        let _ = writeln!(h, "#endif");
        h
    }

    fn collect_decls(&self) -> (Vec<ast::FunctionDecl>, Vec<ast::ConstantDecl>) {
        let mut fns = Vec::new();
        let mut consts = Vec::new();
        self.collect_items(&self.module.ast.items, &mut fns, &mut consts);
        (fns, consts)
    }

    fn collect_items(
        &self,
        items: &[ast::Item],
        fns: &mut Vec<ast::FunctionDecl>,
        consts: &mut Vec<ast::ConstantDecl>,
    ) {
        for item in items {
            match &item.kind {
                ast::ItemKind::Function(f) => {
                    if f.type_parameters.is_empty() {
                        fns.push(f.clone());
                    }
                }
                ast::ItemKind::Constant(c) => consts.push(c.clone()),
                ast::ItemKind::ScopedItems(s) => self.collect_items(&s.items, fns, consts),
                ast::ItemKind::Export(exp) => {
                    if let ast::ExportForm::DefaultFunction(f) = &exp.form
                        && f.type_parameters.is_empty()
                    {
                        fns.push(f.clone());
                    }
                }
                _ => {}
            }
        }
    }

    fn find_main(&self) -> Option<ast::FunctionDecl> {
        for item in &self.module.ast.items {
            if let ast::ItemKind::Function(f) = &item.kind
                && f.name == "main"
                && f.parameters.is_empty()
                && f.error_set.is_empty()
            {
                let ret = f.return_type.as_ref().map(to_type).unwrap_or(Type::Unit);
                if ret == Type::Unit || ret == Type::I32 {
                    return Some(f.clone());
                }
            }
            if let ast::ItemKind::ScopedItems(s) = &item.kind {
                for inner in &s.items {
                    if let ast::ItemKind::Function(f) = &inner.kind
                        && f.name == "main"
                        && f.parameters.is_empty()
                        && f.error_set.is_empty()
                    {
                        let ret = f.return_type.as_ref().map(to_type).unwrap_or(Type::Unit);
                        if ret == Type::Unit || ret == Type::I32 {
                            return Some(f.clone());
                        }
                    }
                }
            }
        }
        None
    }

    fn emit_fn_proto(&mut self, f: &ast::FunctionDecl) {
        if !f.type_parameters.is_empty() {
            return;
        }
        let sym = self.fn_symbol(&f.name);
        self.emit_fn_proto_named(f, &sym);
    }

    fn emit_fn_proto_named(&mut self, f: &ast::FunctionDecl, sym: &str) {
        let (ret_c, _is_fallible) = self.fn_return_c(f);
        let params = self.fn_params_c(f);
        if self.linked {
            let _ = writeln!(self.out, "{ret_c} {sym}({params});");
        } else {
            let _ = writeln!(self.out, "static {ret_c} {sym}({params});");
        }
    }

    fn fn_params_c(&mut self, f: &ast::FunctionDecl) -> String {
        if f.parameters.is_empty() {
            return "void".to_string();
        }
        f.parameters
            .iter()
            .map(|p| {
                let t = self.tcx.c_type(&to_type(&p.ty));
                format!("{t} v_{}", sanitize(&p.name))
            })
            .collect::<Vec<_>>()
            .join(", ")
    }

    fn fn_return_c(&mut self, f: &ast::FunctionDecl) -> (String, bool) {
        let ret = f.return_type.as_ref().map(to_type).unwrap_or(Type::Unit);
        let errs: Vec<Type> = f.error_set.iter().map(path_to_type).collect();
        if errs.len() > 1 {
            self.tcx.rejects.push(reject(
                "NS8019",
                format!(
                    "function '{}' has multiple error types, unsupported by the C backend first slice",
                    f.name
                ),
            ));
            return ("void".to_string(), false);
        }
        let err = errs.into_iter().next();
        match err {
            Some(e) => {
                let ty = Type::Result(Box::new(ret.clone()), Box::new(e));
                (self.tcx.c_type(&ty), true)
            }
            None => {
                if ret == Type::Unit {
                    ("void".to_string(), false)
                } else {
                    (self.tcx.c_type(&ret), false)
                }
            }
        }
    }

    fn script_export_signature(&mut self, exp: &ast::ScriptExport) -> (String, String) {
        let ret = exp.return_type.as_ref().map(to_type).unwrap_or(Type::Unit);
        let errs: Vec<Type> = exp.error_set.iter().map(path_to_type).collect();
        if errs.len() > 1 {
            self.tcx.rejects.push(reject(
                "NS8019",
                format!(
                    "script export '{}' has multiple error types, unsupported by the C backend",
                    exp.name
                ),
            ));
        }
        let ret_c = if let Some(e) = errs.into_iter().next() {
            let ty = Type::Result(Box::new(ret), Box::new(e));
            self.tcx.c_type(&ty)
        } else if ret == Type::Unit {
            "void".to_string()
        } else {
            self.tcx.c_type(&ret)
        };
        let params_c = if exp.parameters.is_empty() {
            "void".to_string()
        } else {
            exp.parameters
                .iter()
                .map(|p| {
                    let t = self.tcx.c_type(&to_type(&p.ty));
                    format!("{t} {}", sanitize(&p.name))
                })
                .collect::<Vec<_>>()
                .join(", ")
        };
        (ret_c, params_c)
    }

    fn emit_scripts(&mut self) -> Vec<(String, String)> {
        let mut script_sources = Vec::new();
        for s in &self.scripts.clone() {
            let ext = if s.language == "cpp" { "cpp" } else { "c" };
            let filename = format!("script_{}.{ext}", sanitize(&s.name));
            let mut content = String::new();
            if s.language == "cpp" {
                content.push_str("extern \"C\" {\n");
                content.push_str(crate::runtime::RUNTIME_HEADER);
                content.push('\n');
                content
                    .push_str("#include <stdint.h>\n#include <stdbool.h>\n#include <stddef.h>\n\n");
                for exp in &s.exports {
                    let (ret_c, params_c) = self.script_export_signature(exp);
                    let _ = writeln!(content, "{ret_c} {}({params_c});", sanitize(&exp.name));
                }
                content.push_str("}\n\n");
            } else {
                content.push_str(crate::runtime::RUNTIME_HEADER);
                content.push('\n');
                content
                    .push_str("#include <stdint.h>\n#include <stdbool.h>\n#include <stddef.h>\n\n");
            }
            content.push_str(&s.raw_source);
            if !content.ends_with('\n') {
                content.push('\n');
            }
            script_sources.push((filename, content));
        }
        script_sources
    }

    fn emit_constant(&mut self, c: &ast::ConstantDecl) {
        let ty = to_type(&c.ty);
        let ty_c = self.tcx.c_type(&ty);
        self.pre.clear();
        let v = self.expr(&c.value);
        self.flush_pre();
        let _ = writeln!(
            self.out,
            "static const {ty_c} const_{} = {v};",
            sanitize(&c.name)
        );
    }

    fn count_root_slots(&mut self, block: &ast::Block, params: &[ast::Parameter]) -> usize {
        let mut total = 0;
        for p in params {
            let ty = to_type(&p.ty);
            total += self.tcx.root_slot_count(&ty);
        }
        for stmt in &block.statements {
            self.count_stmt_slots(stmt, &mut total);
        }
        total
    }

    fn count_stmt_slots(&mut self, stmt: &Stmt, total: &mut usize) {
        match &stmt.kind {
            StmtKind::Let { ty, value, .. } => {
                let val_ty = ty
                    .as_ref()
                    .map(to_type)
                    .or_else(|| self.type_of(value).cloned())
                    .unwrap_or(Type::Unit);
                *total += self.tcx.root_slot_count(&val_ty);
                self.count_expr_slots(value, total);
            }
            StmtKind::Return { value } => {
                if let Some(value) = value {
                    self.count_expr_slots(value, total);
                }
            }
            StmtKind::Break | StmtKind::Continue => {}
            StmtKind::Defer(s) => self.count_stmt_slots(s, total),
            StmtKind::While { condition, body } => {
                self.count_expr_slots(condition, total);
                for stmt in &body.statements {
                    self.count_stmt_slots(stmt, total);
                }
            }
            StmtKind::For { iterable, body, .. } => {
                self.count_expr_slots(iterable, total);
                let iter_ty = self.type_of(iterable).cloned().unwrap_or(Type::Unit);
                let elem_ty = match &iter_ty {
                    Type::Array(t) | Type::Slice(t) => (**t).clone(),
                    Type::Map(k, _) => (**k).clone(),
                    Type::Bytes => Type::U8,
                    _ => Type::Unit,
                };

                *total += self.tcx.root_slot_count(&iter_ty);
                *total += self.tcx.root_slot_count(&elem_ty);
                for stmt in &body.statements {
                    self.count_stmt_slots(stmt, total);
                }
            }
            StmtKind::Expr(e) => self.count_expr_slots(e, total),
            StmtKind::Scoped(s) => {
                for stmt in &s.statements {
                    self.count_stmt_slots(stmt, total);
                }
            }
        }
    }

    fn count_expr_slots(&mut self, e: &Expr, total: &mut usize) {
        match &e.kind {
            ExprKind::Literal(_) | ExprKind::Identifier(_) => {}
            ExprKind::Unary { operand, .. } | ExprKind::Propagate(operand) => {
                self.count_expr_slots(operand, total);
            }
            ExprKind::Binary { left, right, .. } => {
                self.count_expr_slots(left, total);
                self.count_expr_slots(right, total);
            }
            ExprKind::Assign { target, value } => {
                self.count_expr_slots(target, total);
                self.count_expr_slots(value, total);
            }
            ExprKind::Call { callee, arguments } => {
                self.count_expr_slots(callee, total);
                for argument in arguments {
                    let ty = self.type_of(argument).cloned().unwrap_or(Type::Unit);
                    *total += self.tcx.root_slot_count(&ty);
                    self.count_expr_slots(argument, total);
                }
            }
            ExprKind::Index { target, index } => {
                self.count_expr_slots(target, total);
                self.count_expr_slots(index, total);
            }
            ExprKind::Member { target, .. } | ExprKind::TypeArguments { target, .. } => {
                self.count_expr_slots(target, total);
            }
            ExprKind::Array(items) | ExprKind::Tuple(items) => {
                for item in items {
                    let ty = self.type_of(item).cloned().unwrap_or(Type::Unit);
                    *total += self.tcx.root_slot_count(&ty);
                    self.count_expr_slots(item, total);
                }
            }
            ExprKind::Map(entries) => {
                for entry in entries {
                    self.count_expr_slots(&entry.key, total);
                    self.count_expr_slots(&entry.value, total);
                }
            }
            ExprKind::StructInit { fields, .. } => {
                for field in fields {
                    let ty = self.type_of(&field.value).cloned().unwrap_or(Type::Unit);
                    *total += self.tcx.root_slot_count(&ty);
                    self.count_expr_slots(&field.value, total);
                }
            }
            ExprKind::Block(block) => {
                for stmt in &block.statements {
                    self.count_stmt_slots(stmt, total);
                }
            }
            ExprKind::If {
                condition,
                then_branch,
                else_branch,
            } => {
                self.count_expr_slots(condition, total);
                for stmt in &then_branch.statements {
                    self.count_stmt_slots(stmt, total);
                }
                match else_branch {
                    Some(ElseBranch::Block(block)) => {
                        for stmt in &block.statements {
                            self.count_stmt_slots(stmt, total);
                        }
                    }
                    Some(ElseBranch::If(inner)) => self.count_expr_slots(inner, total),
                    None => {}
                }
            }
            ExprKind::Match { value, arms } => {
                self.count_expr_slots(value, total);
                for arm in arms {
                    match &arm.body {
                        MatchArmBody::Expr(expr) => self.count_expr_slots(expr, total),
                        MatchArmBody::Block(block) => {
                            for stmt in &block.statements {
                                self.count_stmt_slots(stmt, total);
                            }
                        }
                    }
                }
            }
            ExprKind::Spawn { body: _ } => {
                let task_ty = self
                    .type_of(e)
                    .cloned()
                    .unwrap_or_else(|| Type::Named("Task".to_owned(), vec![Type::Unit]));
                *total += self.tcx.root_slot_count(&task_ty);
            }
            ExprKind::Fail { arguments, .. } => match arguments {
                FailArguments::Tuple(items) => {
                    for item in items {
                        self.count_expr_slots(item, total);
                    }
                }
                FailArguments::Struct(fields) => {
                    for field in fields {
                        self.count_expr_slots(&field.value, total);
                    }
                }
            },
            ExprKind::Panic { message } | ExprKind::Abort { message } => {
                self.count_expr_slots(message, total);
            }
        }
    }

    fn publish_local(&mut self, base: &str, ty: &Type, first: usize) -> usize {
        let Some(var) = self.roots.as_ref().map(|frame| frame.var.clone()) else {
            return 0;
        };
        let slots = self.tcx.root_exprs(base, ty);
        for (offset, slot) in slots.iter().enumerate() {
            let _ = writeln!(
                self.out,
                "    astra_gc_publish({var} + {}, {slot});",
                first + offset
            );
        }
        slots.len()
    }

    fn record_local(&mut self, name: String, ty: Type, first: usize, slots: usize) {
        if let Some(frame) = self.roots.as_mut() {
            frame.used += slots;
            frame.all_locals.push((name.clone(), ty.clone()));
            if slots > 0 {
                frame.locals.push((name, ty, first));
            }
        }
    }

    fn push_capture_scope(&mut self) {
        if let Some(frame) = self.roots.as_mut() {
            frame
                .capture_scopes
                .push((frame.all_locals.len(), frame.locals.len()));
        }
    }

    fn pop_capture_scope(&mut self) {
        if let Some(frame) = self.roots.as_mut()
            && let Some((all_len, local_len)) = frame.capture_scopes.pop()
        {
            frame.all_locals.truncate(all_len);
            frame.locals.truncate(local_len);
        }
    }

    fn republish_local(&mut self, name: &str) {
        let Some(frame) = self.roots.as_ref() else {
            return;
        };
        let Some((c_name, ty, first)) = frame
            .locals
            .iter()
            .rev()
            .find(|(local, _, _)| local == name)
            .cloned()
        else {
            return;
        };
        self.publish_local(&c_name, &ty, first);
    }

    fn root_base_name(&self, e: &Expr) -> Option<String> {
        match &e.kind {
            ExprKind::Identifier(name) => Some(format!("v_{}", sanitize(name))),
            ExprKind::Member { target, .. }
            | ExprKind::Index { target, .. }
            | ExprKind::TypeArguments { target, .. }
            | ExprKind::Propagate(target) => self.root_base_name(target),
            _ => None,
        }
    }

    fn mutation_bases(&self, e: &Expr, out: &mut Vec<String>) {
        match &e.kind {
            ExprKind::Literal(_) | ExprKind::Identifier(_) => {}
            ExprKind::Unary { operand, .. } | ExprKind::Propagate(operand) => {
                self.mutation_bases(operand, out);
            }
            ExprKind::Binary { left, right, .. } => {
                self.mutation_bases(left, out);
                self.mutation_bases(right, out);
            }
            ExprKind::Assign { target, value } => {
                if let Some(name) = self.root_base_name(target) {
                    out.push(name);
                }
                self.mutation_bases(target, out);
                self.mutation_bases(value, out);
            }
            ExprKind::Call { callee, arguments } => {
                if let ExprKind::Member { target, .. } = &callee.kind
                    && let Some(name) = self.root_base_name(target)
                {
                    out.push(name);
                }
                self.mutation_bases(callee, out);
                for argument in arguments {
                    self.mutation_bases(argument, out);
                }
            }
            ExprKind::Index { target, index } => {
                self.mutation_bases(target, out);
                self.mutation_bases(index, out);
            }
            ExprKind::Member { target, .. } | ExprKind::TypeArguments { target, .. } => {
                self.mutation_bases(target, out);
            }
            ExprKind::Array(items) | ExprKind::Tuple(items) => {
                for item in items {
                    self.mutation_bases(item, out);
                }
            }
            ExprKind::Map(entries) => {
                for entry in entries {
                    self.mutation_bases(&entry.key, out);
                    self.mutation_bases(&entry.value, out);
                }
            }
            ExprKind::StructInit { fields, .. } => {
                for field in fields {
                    self.mutation_bases(&field.value, out);
                }
            }
            ExprKind::Block(block) => {
                for stmt in &block.statements {
                    self.mutation_bases_stmt(stmt, out);
                }
            }
            ExprKind::If {
                condition,
                then_branch,
                else_branch,
            } => {
                self.mutation_bases(condition, out);
                for stmt in &then_branch.statements {
                    self.mutation_bases_stmt(stmt, out);
                }
                match else_branch {
                    Some(ElseBranch::Block(block)) => {
                        for stmt in &block.statements {
                            self.mutation_bases_stmt(stmt, out);
                        }
                    }
                    Some(ElseBranch::If(inner)) => self.mutation_bases(inner, out),
                    None => {}
                }
            }
            ExprKind::Match { value, arms } => {
                self.mutation_bases(value, out);
                for arm in arms {
                    match &arm.body {
                        MatchArmBody::Expr(expr) => self.mutation_bases(expr, out),
                        MatchArmBody::Block(block) => {
                            for stmt in &block.statements {
                                self.mutation_bases_stmt(stmt, out);
                            }
                        }
                    }
                }
            }
            ExprKind::Spawn { body } => {
                for stmt in &body.statements {
                    self.mutation_bases_stmt(stmt, out);
                }
            }
            ExprKind::Fail { arguments, .. } => match arguments {
                FailArguments::Tuple(items) => {
                    for item in items {
                        self.mutation_bases(item, out);
                    }
                }
                FailArguments::Struct(fields) => {
                    for field in fields {
                        self.mutation_bases(&field.value, out);
                    }
                }
            },
            ExprKind::Panic { message } | ExprKind::Abort { message } => {
                self.mutation_bases(message, out);
            }
        }
    }

    fn mutation_bases_stmt(&self, stmt: &Stmt, out: &mut Vec<String>) {
        match &stmt.kind {
            StmtKind::Let { value, .. } | StmtKind::Expr(value) => {
                self.mutation_bases(value, out);
            }
            StmtKind::Return { value } => {
                if let Some(value) = value {
                    self.mutation_bases(value, out);
                }
            }
            StmtKind::Break | StmtKind::Continue => {}
            StmtKind::Defer(s) => self.mutation_bases_stmt(s, out),
            StmtKind::While { condition, body } => {
                self.mutation_bases(condition, out);
                for stmt in &body.statements {
                    self.mutation_bases_stmt(stmt, out);
                }
            }
            StmtKind::For { iterable, body, .. } => {
                self.mutation_bases(iterable, out);
                for stmt in &body.statements {
                    self.mutation_bases_stmt(stmt, out);
                }
            }
            StmtKind::Scoped(s) => {
                for stmt in &s.statements {
                    self.mutation_bases_stmt(stmt, out);
                }
            }
        }
    }

    fn republish_mutations(&mut self, e: &Expr) {
        let mut bases = Vec::new();
        self.mutation_bases(e, &mut bases);
        for base in bases {
            self.republish_local(&base);
        }
    }

    fn emit_frame_exit(&mut self) {
        if let Some(frame) = &self.roots {
            let var = frame.var.clone();
            self.writeln_s(&format!("    astra_gc_frame_exit({var});"));
        }
    }

    fn task_join_code(&mut self, task: &PendingTask) -> String {
        let inner = task_value_type(&task.task_ty);
        let inner_c = self.tcx.c_type(&inner);
        let shape = self.tcx.value_shape_ref(&inner);
        let var = &task.var;
        let mut code = String::new();
        let _ = writeln!(code, "    if (!astra_task_join(&{var}->core)) {{");
        let _ = writeln!(code, "        astra_task_raise(&{var}->core);");
        let _ = writeln!(code, "    }}");
        let _ = writeln!(code, "    if ({var}->core.heap) {{");
        let _ = writeln!(
            code,
            "        astra_gc_copy_into(astra_current_heap(), {var}->core.heap, {shape}, sizeof({inner_c}), &{var}->value, (char*){var}->core.closure + {var}->core.value_offset);"
        );
        let _ = writeln!(code, "        astra_task_heap_free(&{var}->core);");
        let _ = writeln!(code, "    }}");
        code
    }

    fn emit_scope_task_joins(&mut self) {
        let tasks = self.scope_tasks.last().cloned().unwrap_or_default();
        for task in &tasks {
            let code = self.task_join_code(task);
            self.out.push_str(&code);
        }
    }

    fn emit_scope_task_joins_into_pre(&mut self) {
        let tasks = self.scope_tasks.last().cloned().unwrap_or_default();
        for task in &tasks {
            let code = self.task_join_code(task);
            self.pre.push_str(&code);
        }
    }

    fn emit_all_task_joins(&mut self) {
        let scopes = self.scope_tasks.clone();
        for scope in scopes.iter().rev() {
            for task in scope {
                let code = self.task_join_code(task);
                self.out.push_str(&code);
            }
        }
    }

    fn returns_across_abi(&self, f: &ast::FunctionDecl) -> bool {
        self.library
            && f.type_parameters.is_empty()
            && self
                .module
                .hir
                .items
                .iter()
                .any(|item| item.name == f.name && item.exported)
    }

    fn emit_pin_roots(&mut self, base: &str, ty: &Type) {
        for slot in self.tcx.root_exprs(base, ty) {
            let _ = writeln!(self.out, "    astra_gc_pin({slot});");
        }
    }

    fn emit_function(&mut self, f: &ast::FunctionDecl) {
        let sym = self.fn_symbol(&f.name);
        self.emit_function_named(f, &sym);
    }

    fn emit_function_named(&mut self, f: &ast::FunctionDecl, sym: &str) {
        let ret = f.return_type.as_ref().map(to_type).unwrap_or(Type::Unit);
        let errs: Vec<Type> = f.error_set.iter().map(path_to_type).collect();
        let err = errs.into_iter().next();
        self.cur_ret = ret.clone();
        self.cur_err = err.clone();
        self.pin_returns = self.returns_across_abi(f);
        let (ret_c, is_fallible) = self.fn_return_c(f);
        let params = self.fn_params_c(f);
        let root_slots = self.count_root_slots(&f.body, &f.parameters);
        if self.linked {
            let _ = writeln!(self.out, "{ret_c} {sym}({params}) {{");
        } else {
            let _ = writeln!(self.out, "static {ret_c} {sym}({params}) {{");
        }

        self.roots = Some(RootFrame {
            var: "_rf".to_string(),
            used: 0,
            locals: Vec::new(),
            all_locals: Vec::new(),
            capture_scopes: Vec::new(),
        });
        let _ = writeln!(
            self.out,
            "    size_t _rf = astra_gc_frame_enter({root_slots});"
        );
        for parameter in &f.parameters {
            let ty = to_type(&parameter.ty);
            let c_name = format!("v_{}", sanitize(&parameter.name));
            let first = self.roots.as_ref().map_or(0, |frame| frame.used);
            let slots = self.publish_local(&c_name, &ty, first);
            self.record_local(c_name, ty, first, slots);
        }
        self.writeln_s("    astra_gc_maybe_collect();");
        self.emit_block_body(&f.body, true, is_fallible);
        self.emit_frame_exit();
        self.roots = None;
        self.pin_returns = false;
        self.out.push_str("}\n");
    }

    fn emit_main(&mut self, main: &ast::FunctionDecl) {
        let ret = main.return_type.as_ref().map(to_type).unwrap_or(Type::Unit);
        let sym = self.fn_symbol("main");
        self.out.push_str("int main(void) {\n");
        self.out.push_str("    int astra_exit_code = 0;\n");
        if ret == Type::I32 {
            let _ = writeln!(self.out, "    astra_exit_code = (int){sym}();");
        } else {
            let _ = writeln!(self.out, "    {sym}();");
        }

        self.out.push_str("    astra_runtime_shutdown();\n");
        self.out.push_str("    return astra_exit_code;\n");
        self.out.push_str("}\n");
    }

    fn emit_block_body(&mut self, block: &ast::Block, is_fn_body: bool, is_fallible: bool) {
        self.defers.push(Vec::new());
        self.scope_tasks.push(Vec::new());
        self.push_capture_scope();
        let n = block.statements.len();
        for (i, stmt) in block.statements.iter().enumerate() {
            let is_tail = is_fn_body && i + 1 == n;
            if is_tail {
                self.emit_tail_stmt(stmt, is_fallible);
            } else {
                self.emit_stmt(stmt);
            }
        }
        if is_fn_body && block.statements.is_empty() {
            self.emit_implicit_tail_return(is_fallible);
        } else if !is_fn_body {
            self.emit_scope_task_joins();
            self.emit_scope_defers();
            self.emit_scope_task_joins();
        }
        self.scope_tasks.pop();
        self.defers.pop();
        self.pop_capture_scope();
    }

    fn emit_implicit_tail_return(&mut self, is_fallible: bool) {
        self.emit_all_defers();
        self.emit_frame_exit();
        if is_fallible {
            let ret_c = self.fn_result_c();
            let _ = writeln!(self.out, "    return ({ret_c}){{ .ok = true }};");
        }
    }

    fn emit_tail_stmt(&mut self, stmt: &Stmt, is_fallible: bool) {
        match &stmt.kind {
            StmtKind::Expr(e) => self.emit_tail_expr(e, is_fallible),
            StmtKind::Return { .. } => self.emit_stmt(stmt),
            _ => {
                self.emit_stmt(stmt);
                self.emit_implicit_tail_return(is_fallible);
            }
        }
    }

    fn emit_tail_expr(&mut self, e: &Expr, is_fallible: bool) {
        if self.all_branches_return(e) {
            self.emit_expr_stmt(e);
            return;
        }
        let ty = self.type_of(e).cloned();
        if matches!(ty, Some(Type::Never)) {
            self.emit_expr_stmt(e);
            return;
        }
        self.pre.clear();
        let v = self.expr(e);
        self.flush_pre();
        if is_fallible {
            let ret_c = self.fn_result_c();
            let result_ty = Type::Result(
                Box::new(self.cur_ret.clone()),
                Box::new(
                    self.cur_err
                        .clone()
                        .expect("fallible function has an error"),
                ),
            );
            self.emit_all_defers();
            if self.pin_returns {
                let tmp = self.freshid("ret");
                let _ = writeln!(
                    self.out,
                    "    {ret_c} {tmp} = ({ret_c}){{ .ok = true, .value = {v} }};"
                );
                self.emit_pin_roots(&tmp, &result_ty);
                self.emit_frame_exit();
                let _ = writeln!(self.out, "    return {tmp};");
            } else {
                self.emit_frame_exit();
                let _ = writeln!(
                    self.out,
                    "    return ({ret_c}){{ .ok = true, .value = {v} }};"
                );
            }
        } else if self.cur_ret == Type::Unit {
            self.emit_value_stmt(e, v, &ty);
            self.emit_all_defers();
        } else {
            self.emit_all_defers();
            if self.pin_returns {
                let ret_ty = self.cur_ret.clone();
                let ret_c = self.tcx.c_type(&ret_ty);
                let tmp = self.freshid("ret");
                let _ = writeln!(self.out, "    {ret_c} {tmp} = {v};");
                self.emit_pin_roots(&tmp, &ret_ty);
                self.emit_frame_exit();
                let _ = writeln!(self.out, "    return {tmp};");
            } else {
                self.emit_frame_exit();
                let _ = writeln!(self.out, "    return {v};");
            }
        }
    }

    fn emit_all_defers(&mut self) {
        self.emit_all_task_joins();
        let all: Vec<ast::Stmt> = self
            .defers
            .iter()
            .rev()
            .flat_map(|scope| scope.iter().rev().cloned())
            .collect();
        for d in &all {
            self.emit_stmt(d);
        }

        self.emit_all_task_joins();
    }

    fn emit_scope_defers(&mut self) {
        if let Some(scope) = self.defers.last() {
            let defers: Vec<ast::Stmt> = scope.iter().rev().cloned().collect();
            for d in &defers {
                self.emit_stmt(d);
            }
        }
    }

    fn emit_scope_defers_into_pre(&mut self) {
        if let Some(scope) = self.defers.last() {
            let defers: Vec<ast::Stmt> = scope.iter().rev().cloned().collect();
            for d in &defers {
                self.emit_stmt_into_pre(d);
            }
        }
    }

    fn all_branches_return(&self, e: &Expr) -> bool {
        match &e.kind {
            ExprKind::If {
                then_branch,
                else_branch,
                ..
            } => {
                let then_ret = self.block_diverges(then_branch);
                let else_ret = match else_branch {
                    Some(ElseBranch::Block(b)) => self.block_diverges(b),
                    Some(ElseBranch::If(inner)) => self.all_branches_return(inner),
                    None => false,
                };
                then_ret && else_ret
            }
            ExprKind::Block(b) => self.block_diverges(b),
            _ => false,
        }
    }

    fn block_diverges(&self, block: &ast::Block) -> bool {
        match block.statements.last().map(|s| &s.kind) {
            Some(StmtKind::Return { .. }) => true,
            Some(StmtKind::Expr(e)) => matches!(
                e.kind,
                ExprKind::Fail { .. } | ExprKind::Panic { .. } | ExprKind::Abort { .. }
            ),
            _ => false,
        }
    }

    fn fn_result_c(&mut self) -> String {
        let ret = self.cur_ret.clone();
        match &self.cur_err {
            Some(err) => self
                .tcx
                .c_type(&Type::Result(Box::new(ret), Box::new(err.clone()))),
            None => self.tcx.c_type(&ret),
        }
    }

    fn emit_stmt(&mut self, stmt: &Stmt) {
        match &stmt.kind {
            StmtKind::Let {
                mutable: _,
                name,
                ty,
                value,
            } => self.emit_let(name, ty.as_ref(), value),
            StmtKind::Return { value } => self.emit_return(value.as_ref()),
            StmtKind::Break => {
                self.emit_all_task_joins();
                self.writeln_s("    break;");
            }
            StmtKind::Continue => {
                self.emit_all_task_joins();
                self.writeln_s("    astra_gc_maybe_collect();");
                self.writeln_s("    continue;");
            }
            StmtKind::Defer(s) => {
                if let Some(scope) = self.defers.last_mut() {
                    scope.push((**s).clone());
                }
            }
            StmtKind::While { condition, body } => self.emit_while(condition, body),
            StmtKind::For {
                binding,
                iterable,
                body,
            } => self.emit_for(binding, iterable, body),
            StmtKind::Expr(e) => self.emit_expr_stmt(e),
            StmtKind::Scoped(s) => {
                self.writeln_s("    {");
                self.emit_scoped_block(s);
                self.writeln_s("    }");
            }
        }
    }

    fn emit_scoped_block(&mut self, scoped: &ast::ScopedStatements) {
        self.defers.push(Vec::new());
        self.push_capture_scope();
        for stmt in &scoped.statements {
            self.emit_stmt(stmt);
        }
        self.emit_scope_defers();
        self.pop_capture_scope();
        self.defers.pop();
    }

    fn emit_let(&mut self, name: &str, ty: Option<&ast::Type>, value: &Expr) {
        let val_ty = ty
            .map(to_type)
            .or_else(|| self.type_of(value).cloned())
            .unwrap_or(Type::Unit);
        if val_ty == Type::Unit {
            self.emit_expr_stmt(value);
            return;
        }
        self.pre.clear();
        let v = self.expr_with_expected(value, &val_ty);
        self.flush_pre();
        let ty_c = self.tcx.c_type(&val_ty);
        let _ = writeln!(self.out, "    {ty_c} v_{} = {v};", sanitize(name));
        let c_name = format!("v_{}", sanitize(name));
        let first = self.roots.as_ref().map_or(0, |frame| frame.used);
        let slots = self.publish_local(&c_name, &val_ty, first);
        self.record_local(c_name, val_ty, first, slots);
    }

    fn emit_return(&mut self, value: Option<&Expr>) {
        let is_fallible = self.cur_err.is_some();
        match value {
            None => {
                self.emit_all_defers();
                self.emit_frame_exit();
                if is_fallible {
                    let ret_c = self.fn_result_c();
                    let _ = writeln!(self.out, "    return ({ret_c}){{ .ok = true }};");
                } else {
                    self.writeln_s("    return;");
                }
            }
            Some(e) => {
                let ty = self.type_of(e).cloned();
                if matches!(ty, Some(Type::Never)) {
                    self.emit_expr_stmt(e);
                    return;
                }
                self.pre.clear();
                let expected = self.cur_ret.clone();
                let v = self.expr_with_expected(e, &expected);
                self.flush_pre();
                self.emit_all_defers();
                if is_fallible {
                    let ret_c = self.fn_result_c();
                    let result_ty = Type::Result(
                        Box::new(self.cur_ret.clone()),
                        Box::new(
                            self.cur_err
                                .clone()
                                .expect("fallible function has an error"),
                        ),
                    );
                    if self.pin_returns {
                        let tmp = self.freshid("ret");
                        let _ = writeln!(
                            self.out,
                            "    {ret_c} {tmp} = ({ret_c}){{ .ok = true, .value = {v} }};"
                        );
                        self.emit_pin_roots(&tmp, &result_ty);
                        self.emit_frame_exit();
                        let _ = writeln!(self.out, "    return {tmp};");
                    } else {
                        self.emit_frame_exit();
                        let _ = writeln!(
                            self.out,
                            "    return ({ret_c}){{ .ok = true, .value = {v} }};"
                        );
                    }
                } else if self.pin_returns && self.cur_ret != Type::Unit {
                    let ret_ty = self.cur_ret.clone();
                    let ret_c = self.tcx.c_type(&ret_ty);
                    let tmp = self.freshid("ret");
                    let _ = writeln!(self.out, "    {ret_c} {tmp} = {v};");
                    self.emit_pin_roots(&tmp, &ret_ty);
                    self.emit_frame_exit();
                    let _ = writeln!(self.out, "    return {tmp};");
                } else {
                    self.emit_frame_exit();
                    let _ = writeln!(self.out, "    return {v};");
                }
            }
        }
    }

    fn emit_while(&mut self, condition: &Expr, body: &ast::Block) {
        self.pre.clear();

        let saved = self.hoist;
        self.hoist = false;
        let cond = self.expr(condition);
        self.hoist = saved;
        self.flush_pre();
        self.republish_mutations(condition);
        let _ = writeln!(self.out, "    while ({cond}) {{");
        self.emit_block_body(body, false, false);
        self.writeln_s("        astra_gc_maybe_collect();");
        self.writeln_s("    }");
    }

    fn emit_for(&mut self, binding: &str, iterable: &Expr, body: &ast::Block) {
        let iter_ty = self.type_of(iterable).cloned().unwrap_or(Type::Unit);
        enum IterKind {
            Array,
            Map,
            Bytes,
        }
        let (elem_ty, kind) = match &iter_ty {
            Type::Array(t) | Type::Slice(t) => ((**t).clone(), IterKind::Array),
            Type::Map(k, _) => ((**k).clone(), IterKind::Map),
            Type::Bytes => (Type::U8, IterKind::Bytes),
            _ => {
                self.tcx.rejects.push(reject(
                    "NS8021",
                    "for-loop iteration is only supported over arrays, maps, and bytes in the C backend first slice",
                ));
                return;
            }
        };
        self.pre.clear();
        let iter = self.expr(iterable);
        self.flush_pre();
        let iter_var = self.fresh_id("it");
        let iter_c_ty = self.tcx.c_type(&iter_ty);
        let _ = writeln!(self.out, "    {iter_c_ty} {iter_var} = {iter};");

        let first = self.roots.as_ref().map_or(0, |frame| frame.used);
        let slots = self.publish_local(&iter_var, &iter_ty, first);
        self.record_local(iter_var.clone(), iter_ty, first, slots);
        let elem_c = self.tcx.c_type(&elem_ty);
        let idx = self.fresh_id("i");
        let _ = writeln!(
            self.out,
            "    for (size_t {idx} = 0; {idx} < ({iter_var}).len; ++{idx}) {{"
        );
        match kind {
            IterKind::Map => {
                let _ = writeln!(
                    self.out,
                    "        {elem_c} v_{} = *({elem_c}*)astra_map_key_at(&({iter_var}), {idx});",
                    sanitize(binding)
                );
            }
            IterKind::Array => {
                let _ = writeln!(
                    self.out,
                    "        {elem_c} v_{} = *({elem_c}*)astra_array_index(&({iter_var}), {idx}, sizeof({elem_c}));",
                    sanitize(binding)
                );
            }
            IterKind::Bytes => {
                let _ = writeln!(
                    self.out,
                    "        {elem_c} v_{} = astra_bytes_get({iter_var}, {idx});",
                    sanitize(binding)
                );
            }
        }
        let binding_c = format!("v_{}", sanitize(binding));
        let first = self.roots.as_ref().map_or(0, |frame| frame.used);
        let slots = self.publish_local(&binding_c, &elem_ty, first);
        self.push_capture_scope();
        self.record_local(binding_c, elem_ty, first, slots);
        self.emit_block_body(body, false, false);
        self.writeln_s("        astra_gc_maybe_collect();");
        self.writeln_s("    }");
        self.pop_capture_scope();
    }

    fn emit_expr_stmt(&mut self, e: &Expr) {
        if let ExprKind::If {
            condition,
            then_branch,
            else_branch,
        } = &e.kind
        {
            self.emit_if_stmt(condition, then_branch, else_branch.as_ref());
            return;
        }
        let ty = self.type_of(e).cloned();
        if matches!(ty, Some(Type::Never)) {
            self.pre.clear();
            let _ = self.expr(e);
            self.flush_pre();
            return;
        }
        self.pre.clear();
        let v = self.expr(e);
        self.flush_pre();
        self.emit_value_stmt(e, v, &ty);
        self.republish_mutations(e);
    }

    fn emit_if_stmt(
        &mut self,
        condition: &Expr,
        then_branch: &ast::Block,
        else_branch: Option<&ElseBranch>,
    ) {
        self.pre.clear();
        let cond = self.expr(condition);
        self.flush_pre();
        self.republish_mutations(condition);
        let _ = writeln!(self.out, "    if ({cond}) {{");
        self.push_capture_scope();
        for stmt in &then_branch.statements {
            self.emit_stmt(stmt);
        }
        self.pop_capture_scope();
        self.emit_else_branch(else_branch);
    }

    fn emit_else_branch(&mut self, else_branch: Option<&ElseBranch>) {
        match else_branch {
            Some(ElseBranch::Block(b)) => {
                self.writeln_s("    } else {");
                self.push_capture_scope();
                for stmt in &b.statements {
                    self.emit_stmt(stmt);
                }
                self.pop_capture_scope();
                self.writeln_s("    }");
            }
            Some(ElseBranch::If(inner)) => {
                if let ExprKind::If {
                    condition: inner_cond,
                    then_branch: inner_then,
                    else_branch: inner_else,
                } = &inner.kind
                {
                    self.pre.clear();
                    let icond = self.expr(inner_cond);
                    self.flush_pre();
                    let _ = writeln!(self.out, "    }} else if ({icond}) {{");
                    self.push_capture_scope();
                    for stmt in &inner_then.statements {
                        self.emit_stmt(stmt);
                    }
                    self.pop_capture_scope();
                    self.emit_else_branch(inner_else.as_ref());
                } else {
                    self.writeln_s("    } else {");
                    self.push_capture_scope();
                    self.emit_expr_stmt(inner);
                    self.pop_capture_scope();
                    self.writeln_s("    }");
                }
            }
            None => self.writeln_s("    }"),
        }
    }

    fn emit_value_stmt(&mut self, e: &Expr, v: String, ty: &Option<Type>) {
        let is_void_call = matches!(e.kind, ExprKind::Call { .. } | ExprKind::Assign { .. })
            && matches!(ty, Some(Type::Unit));
        if is_void_call {
            let _ = writeln!(self.out, "    {v};");
        } else {
            let _ = writeln!(self.out, "    (void)({v});");
        }
    }

    fn flush_pre(&mut self) {
        self.out.push_str(&self.pre);
        self.pre.clear();
    }

    fn writeln_s(&mut self, line: &str) {
        self.out.push_str(line);
        self.out.push('\n');
    }

    fn fresh_id(&mut self, prefix: &str) -> String {
        self.fresh += 1;
        format!("__{prefix}{}", self.fresh)
    }

    fn type_of(&self, e: &Expr) -> Option<&Type> {
        self.ann.type_of(e)
    }

    fn expr_expecting(&mut self, e: &Expr, expected: &Type) -> String {
        let previous = self.expected.replace(expected.clone());
        let out = self.expr(e);
        self.expected = previous;
        out
    }

    fn expr_with_expected(&mut self, e: &Expr, expected: &Type) -> String {
        if mentions_unresolved(expected) {
            self.expr(e)
        } else {
            self.expr_expecting(e, expected)
        }
    }

    fn c_type(&mut self, t: &Type) -> String {
        self.tcx.c_type(t)
    }

    fn zero(&mut self, t: &Type) -> String {
        match t {
            Type::Bool => "false".into(),
            Type::F32 | Type::F64 => "0.0".into(),
            Type::I8
            | Type::I16
            | Type::I32
            | Type::I64
            | Type::U8
            | Type::U16
            | Type::U32
            | Type::U64
            | Type::Usize
            | Type::Isize => "0".into(),
            Type::Unit => "(NS_Unit){0}".into(),
            Type::Str => "(NS_Str){0}".into(),
            Type::Bytes => "(NS_Bytes){0}".into(),
            Type::Array(_) | Type::Slice(_) => "(NS_Array){0}".into(),
            Type::Map(_, _) => "(NS_Map){0}".into(),
            Type::Option(_) => format!("({}){{ .some = false }}", self.c_type(t)),
            Type::Result(_, _) => format!("({}){{ .ok = true }}", self.c_type(t)),
            _ => format!("(({}){{0}})", self.c_type(t)),
        }
    }

    fn expr(&mut self, e: &Expr) -> String {
        match &e.kind {
            ExprKind::Literal(lit) => self.literal_c(lit),
            ExprKind::Identifier(name) => self.identifier_c(name, e),
            ExprKind::Unary { operator, operand } => self.unary_c(*operator, operand),
            ExprKind::Binary {
                operator,
                left,
                right,
            } => self.binary_c(*operator, left, right, e),
            ExprKind::Assign { target, value } => self.assign_c(target, value),
            ExprKind::Call { callee, arguments } => self.call_c(callee, arguments, e),
            ExprKind::Index { target, index } => self.index_c(target, index, e),
            ExprKind::Member { target, member } => self.member_c(target, member, e),
            ExprKind::TypeArguments { target, .. } => self.expr(target),
            ExprKind::Propagate(inner) => self.propagate_c(inner),
            ExprKind::Array(elements) => self.array_literal_c(elements, e),
            ExprKind::Map(entries) => self.map_literal_c(entries, e),
            ExprKind::Tuple(elements) => self.tuple_c(elements, e),
            ExprKind::StructInit { path, fields } => self.struct_init_c(path, fields),
            ExprKind::Block(block) => self.block_expr_c(block),
            ExprKind::If {
                condition,
                then_branch,
                else_branch,
            } => self.if_expr_c(condition, then_branch, else_branch.as_ref(), e),
            ExprKind::Match { value, arms } => self.match_expr_c(value, arms, e),
            ExprKind::Spawn { body } => self.spawn_c(body, e),
            ExprKind::Fail { error, arguments } => self.fail_c(error, arguments),
            ExprKind::Panic { message } => self.panic_c(message),
            ExprKind::Abort { message } => self.abort_c(message),
        }
    }

    fn literal_c(&mut self, lit: &Literal) -> String {
        match lit {
            Literal::Integer(raw) => {
                let cleaned: String = raw.chars().filter(|c| *c != '_').collect();

                cleaned
                    .strip_suffix("u8")
                    .filter(|s| !s.is_empty() && s.chars().all(|c| c.is_ascii_digit()))
                    .unwrap_or(&cleaned)
                    .to_string()
            }
            Literal::Float(raw) => raw.chars().filter(|c| *c != '_').collect::<String>(),
            Literal::String(value) => self.c_string_literal(value),
            Literal::Bytes(bytes) => self.c_bytes_literal(bytes),
            Literal::Bool(b) => {
                if *b {
                    "true".into()
                } else {
                    "false".into()
                }
            }
            Literal::Unit => "(NS_Unit){0}".into(),
        }
    }

    fn c_string_literal(&self, value: &str) -> String {
        let mut bytes = String::from("\"");
        let mut prev_hex = false;
        for &b in value.as_bytes() {
            match b {
                b'\\' => {
                    bytes.push_str("\\\\");
                    prev_hex = false;
                }
                b'"' => {
                    bytes.push_str("\\\"");
                    prev_hex = false;
                }
                b'\n' => {
                    bytes.push_str("\\n");
                    prev_hex = false;
                }
                b'\r' => {
                    bytes.push_str("\\r");
                    prev_hex = false;
                }
                b'\t' => {
                    bytes.push_str("\\t");
                    prev_hex = false;
                }
                0x20..=0x7e => {
                    bytes.push(b as char);
                    prev_hex = false;
                }
                _ => {
                    let _ = write!(bytes, "\\x{b:02X}");
                    prev_hex = true;
                }
            }
        }
        bytes.push('"');

        let lit = if prev_hex {
            format!("\"\" {bytes}")
        } else {
            bytes
        };
        let len = value.len();
        format!("(NS_Str){{ .data = {lit}, .len = {len} }}")
    }

    fn identifier_c(&mut self, name: &str, _e: &Expr) -> String {
        if name == "None"
            && let Some(expected @ Type::Option(_)) = self.expected.clone()
        {
            let opt_c = self.tcx.c_type(&expected);
            return format!("({opt_c}){{ .some = false }}");
        }

        if self.imports.contains_key(name) || self.default_imports.contains(name) {
            self.tcx.rejects.push(reject(
                "NS8019",
                format!(
                    "cross-module reference to value '{name}' is not supported by the C backend; only cross-module function calls link"
                ),
            ));
            return "0".to_string();
        }

        if self.is_constant(name) {
            format!("const_{}", sanitize(name))
        } else {
            format!("v_{}", sanitize(name))
        }
    }

    fn is_constant(&self, name: &str) -> bool {
        fn in_items(items: &[ast::Item], name: &str) -> bool {
            items.iter().any(|item| match &item.kind {
                ast::ItemKind::Constant(c) => c.name == name,
                ast::ItemKind::ScopedItems(s) => in_items(&s.items, name),
                _ => false,
            })
        }
        in_items(&self.module.ast.items, name)
    }

    fn unary_c(&mut self, op: UnaryOp, operand: &Expr) -> String {
        let v = self.expr(operand);
        match op {
            UnaryOp::Negate => format!("(-{v})"),
            UnaryOp::Not => {
                if matches!(self.type_of(operand), Some(Type::Bool)) {
                    format!("(!{v})")
                } else {
                    format!("(~{v})")
                }
            }
        }
    }

    fn binary_c(&mut self, op: BinaryOp, left: &Expr, right: &Expr, _e: &Expr) -> String {
        if matches!(op, BinaryOp::And | BinaryOp::Or) {
            let l = self.materialize(left);
            let saved = self.hoist;
            self.hoist = false;
            let r = self.expr(right);
            self.hoist = saved;
            return if op == BinaryOp::And {
                format!("({l} && {r})")
            } else {
                format!("({l} || {r})")
            };
        }
        let l = self.materialize(left);
        let r = self.materialize(right);
        match op {
            BinaryOp::Equal | BinaryOp::NotEqual => {
                let ty = self.type_of(left).cloned();
                if matches!(ty, Some(Type::Str)) {
                    if op == BinaryOp::Equal {
                        format!("astra_str_eq({l}, {r})")
                    } else {
                        format!("(!astra_str_eq({l}, {r}))")
                    }
                } else if matches!(ty, Some(Type::Bytes)) {
                    if op == BinaryOp::Equal {
                        format!("astra_bytes_eq({l}, {r})")
                    } else {
                        format!("(!astra_bytes_eq({l}, {r}))")
                    }
                } else if self.eq_supported(left) {
                    let sym = if op == BinaryOp::Equal { "==" } else { "!=" };
                    format!("({l} {sym} {r})")
                } else {
                    self.tcx.rejects.push(reject(
                        "NS8021",
                        "equality is only supported for primitive, str, and bytes values in the C backend first slice",
                    ));
                    "false".into()
                }
            }
            BinaryOp::Less | BinaryOp::LessEqual | BinaryOp::Greater | BinaryOp::GreaterEqual => {
                let sym = match op {
                    BinaryOp::Less => "<",
                    BinaryOp::LessEqual => "<=",
                    BinaryOp::Greater => ">",
                    _ => ">=",
                };
                let ty = self.type_of(left).cloned();
                if matches!(ty, Some(Type::Str)) {
                    format!("(astra_str_cmp({l}, {r}) {sym} 0)")
                } else if matches!(ty, Some(Type::Bytes)) {
                    format!("(astra_bytes_cmp({l}, {r}) {sym} 0)")
                } else {
                    format!("({l} {sym} {r})")
                }
            }
            BinaryOp::Add
            | BinaryOp::Subtract
            | BinaryOp::Multiply
            | BinaryOp::Divide
            | BinaryOp::Remainder
            | BinaryOp::AddWrap
            | BinaryOp::SubtractWrap
            | BinaryOp::MultiplyWrap
            | BinaryOp::DivideWrap => self.arith_c(op, left, right, &l, &r),
            BinaryOp::And | BinaryOp::Or => unreachable!(),
        }
    }

    fn eq_supported(&self, e: &Expr) -> bool {
        matches!(
            self.type_of(e),
            Some(
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
                    | Type::Unit
            )
        )
    }

    fn arith_c(&mut self, op: BinaryOp, left: &Expr, right: &Expr, l: &str, r: &str) -> String {
        let lt = self.type_of(left).cloned();
        let rt = self.type_of(right).cloned();
        let numeric = matches!((&lt, &rt), (Some(a), Some(b)) if a.is_numeric() && b.is_numeric());
        if !numeric {
            self.tcx.rejects.push(reject(
                "NS8021",
                "arithmetic is only supported on numeric values in the C backend first slice",
            ));
            return "0".into();
        }
        let ty = lt.unwrap_or(Type::I32);
        let is_wrapping_op = matches!(
            op,
            BinaryOp::AddWrap
                | BinaryOp::SubtractWrap
                | BinaryOp::MultiplyWrap
                | BinaryOp::DivideWrap
        );
        let policy_wrapping = matches!(self.policy, IntegerPolicy::Wrapping);
        let sym = match op {
            BinaryOp::Add | BinaryOp::AddWrap => "+",
            BinaryOp::Subtract | BinaryOp::SubtractWrap => "-",
            BinaryOp::Multiply | BinaryOp::MultiplyWrap => "*",
            BinaryOp::Divide | BinaryOp::DivideWrap => "/",
            BinaryOp::Remainder => "%",
            _ => unreachable!(),
        };
        if matches!(op, BinaryOp::Divide | BinaryOp::Remainder) || ty.is_float() {
            return format!("({l} {sym} {r})");
        }

        if ty.is_integer() && !is_signed_int(&ty) {
            return format!("({l} {sym} {r})");
        }

        if (is_wrapping_op || policy_wrapping)
            && let Some(width) = signed_width(&ty)
        {
            let helper = match op {
                BinaryOp::Add | BinaryOp::AddWrap => format!("astra_add_{width}"),
                BinaryOp::Subtract | BinaryOp::SubtractWrap => format!("astra_sub_{width}"),
                BinaryOp::Multiply | BinaryOp::MultiplyWrap => format!("astra_mul_{width}"),
                _ => unreachable!(),
            };
            return format!("{helper}({l}, {r})");
        }
        format!("({l} {sym} {r})")
    }

    fn assign_c(&mut self, target: &Expr, value: &Expr) -> String {
        if let ExprKind::Index {
            target: buffer,
            index,
        } = &target.kind
            && matches!(self.type_of(buffer), Some(Type::Bytes))
        {
            let buffer_c = self.expr(buffer);
            let index_c = self.expr(index);
            let value_c = self.expr(value);
            return format!(
                "(astra_bytes_set(&({buffer_c}), {index_c}, (uint8_t)({value_c})), (NS_Unit){{0}})"
            );
        }
        if let ExprKind::Index { target: map, index } = &target.kind
            && let Some(Type::Map(key_ty, val_ty)) = self.type_of(map).cloned()
        {
            let container_ty = Type::Map(key_ty.clone(), val_ty.clone());
            let map_c = self.tcx.c_type(&container_ty);
            let key_c = self.tcx.c_type(&key_ty);
            let val_c = self.tcx.c_type(&val_ty);
            let map_lhs = self.lvalue(map);
            let key = self.expr(index);
            let value = self.expr(value);
            let mtmp = self.freshid("mr");
            let ktmp = self.freshid("mk");
            let vtmp = self.freshid("mv");
            let _ = writeln!(self.pre, "    {map_c}* {mtmp} = &({map_lhs});");
            let _ = writeln!(self.pre, "    {key_c} {ktmp} = {key};");
            let _ = writeln!(self.pre, "    {val_c} {vtmp} = {value};");
            let _ = writeln!(self.pre, "    astra_map_set({mtmp}, &{ktmp}, &{vtmp});");
            return "(NS_Unit){0}".into();
        }
        let lhs = self.lvalue(target);
        let rhs = self.expr(value);
        format!("({lhs} = {rhs})")
    }

    fn lvalue(&mut self, e: &Expr) -> String {
        match &e.kind {
            ExprKind::Identifier(name) => format!("v_{}", sanitize(name)),
            ExprKind::Member { target, member } => {
                let t = self.lvalue(target);
                let f = field_c(member);
                format!("({t}).{f}")
            }
            ExprKind::Index { target, index } => {
                if let Some(Type::Map(key_ty, val_ty)) = self.type_of(target).cloned() {
                    let key_c = self.tcx.c_type(&key_ty);
                    let val_c = self.tcx.c_type(&val_ty);
                    let map_lhs = self.lvalue(target);
                    let key = self.expr(index);
                    let ktmp = self.freshid("mk");
                    let ptr = self.freshid("mv");
                    let _ = writeln!(self.pre, "    {key_c} {ktmp} = {key};");
                    let _ = writeln!(
                        self.pre,
                        "    void* {ptr} = astra_map_get(&({map_lhs}), &{ktmp});"
                    );
                    let _ = writeln!(
                        self.pre,
                        "    if ({ptr} == NULL) {{ astra_panic(\"map key is not present\"); }}"
                    );
                    return format!("(*({val_c}*){ptr})");
                }
                let elem_ty = self.elem_type_of(target);
                let t = self.expr(target);
                let i = self.expr(index);
                let elem_c = self.tcx.c_type(&elem_ty);
                format!("(*({elem_c}*)astra_array_index(&({t}), {i}, sizeof({elem_c})))")
            }
            _ => {
                self.tcx.rejects.push(reject(
                    "NS8021",
                    "this assignment target is not supported by the C backend first slice",
                ));
                "0".into()
            }
        }
    }

    fn elem_type_of(&self, arr: &Expr) -> Type {
        match self.type_of(arr) {
            Some(Type::Array(t)) | Some(Type::Slice(t)) => (**t).clone(),
            _ => Type::I32,
        }
    }

    fn call_c(&mut self, callee: &Expr, args: &[Expr], e: &Expr) -> String {
        let callee = match &callee.kind {
            ExprKind::TypeArguments { target, .. } => target.as_ref(),
            _ => callee,
        };
        match &callee.kind {
            ExprKind::Identifier(name) => {
                match name.as_str() {
                    "Some" => return self.some_c(args, e),
                    "Ok" => return self.ok_c(args, e),
                    "Err" => return self.err_c(args, e),
                    "print" => return self.print_c(args, false),
                    "println" => return self.print_c(args, true),
                    "assert" => return self.assert_c(args),
                    "dbg" => {
                        self.tcx.rejects.push(reject(
                            "NS8021",
                            "dbg is not supported by the C backend first slice",
                        ));
                        return "(NS_Unit){0}".into();
                    }
                    _ => {}
                }

                if let Some(shape) = self.tcx.struct_shape(name)
                    && shape.tuple
                {
                    let ty_c = self.tcx.c_type(&Type::Named(name.to_string(), Vec::new()));
                    let vals: Vec<String> = args.iter().map(|a| self.expr(a)).collect();
                    let joined = vals.join(", ");
                    return format!("({ty_c}){{ {joined} }}");
                }

                if let Some((dep_key, imported)) = self.imports.get(name).cloned() {
                    let arg_c = self.args_c(args);
                    let dep_stem = module_key_stem(&dep_key);
                    return format!("ns_{dep_stem}_{}({arg_c})", sanitize(&imported));
                }

                if self.default_imports.contains(name) {
                    self.tcx.rejects.push(reject(
                        "NS8019",
                        format!(
                            "call to default import '{name}' is not supported by the C backend; import the symbol by name"
                        ),
                    ));
                    return "(NS_Unit){0}".into();
                }

                if let Some(decl) = self.generic_fn(name) {
                    if let Some((symbol, arguments)) = self.instantiate_call(&decl, args) {
                        return format!("{symbol}({arguments})");
                    }
                    return "0".into();
                }

                let arg_c = self.args_c(args);
                format!("{}({arg_c})", self.fn_symbol(name))
            }
            ExprKind::Member { target, member } => {
                if let ExprKind::Identifier(owner) = &target.kind {
                    match (owner.as_str(), member.as_str()) {
                        ("Option", "Some") => return self.some_c(args, e),
                        ("Result", "Ok") => return self.ok_c(args, e),
                        ("Result", "Err") => return self.err_c(args, e),
                        ("Channel", "create") => {
                            let channel_ty = self.type_of(e).cloned().unwrap_or_else(|| {
                                Type::Named("Channel".to_owned(), vec![Type::Unit])
                            });
                            let element = match &channel_ty {
                                Type::Named(_, arguments) => {
                                    arguments.first().cloned().unwrap_or(Type::Unit)
                                }
                                _ => Type::Unit,
                            };
                            let elem_c = self.tcx.c_type(&element);
                            let elem_shape = self.tcx.value_shape_ref(&element);
                            let handle_shape = self.tcx.channel_handle_shape_ref(&element);
                            let data_shape = self.tcx.block_shape_ref(&element);
                            return format!(
                                "astra_channel_new(sizeof({elem_c}), {elem_shape}, {handle_shape}, {data_shape})"
                            );
                        }

                        ("Memory", "alloc") => {
                            let ptr_ty = self
                                .type_of(e)
                                .cloned()
                                .unwrap_or_else(|| Type::Named("Ptr".to_owned(), vec![Type::U8]));
                            let element = match &ptr_ty {
                                Type::Named(_, arguments) => {
                                    arguments.first().cloned().unwrap_or(Type::U8)
                                }
                                _ => Type::U8,
                            };
                            let elem_c = self.tcx.c_type(&element);
                            let count = args
                                .first()
                                .map(|a| self.expr(a))
                                .unwrap_or_else(|| "0".to_owned());
                            return format!(
                                "(({elem_c}*)astra_manual_alloc((size_t)({count}) * sizeof({elem_c})))"
                            );
                        }
                        ("Memory", "free") => {
                            let ptr = args
                                .first()
                                .map(|a| self.expr(a))
                                .unwrap_or_else(|| "NULL".to_owned());
                            return format!("(astra_manual_free({ptr}), (NS_Unit){{0}})");
                        }

                        ("Region", "new") => {
                            let bytes = args
                                .first()
                                .map(|a| self.expr(a))
                                .unwrap_or_else(|| "0".to_owned());
                            return format!("astra_region_new((size_t)({bytes}))");
                        }
                        _ => {
                            if let Some(dep_key) = self.namespaces.get(owner).cloned() {
                                let arg_c = self.args_c(args);
                                let dep_stem = module_key_stem(&dep_key);
                                return format!("ns_{dep_stem}_{}({arg_c})", sanitize(member));
                            }

                            if self.tcx.enum_shape(owner).is_some() {
                                return self.enum_variant_c(owner, member, args);
                            }

                            if self.scripts.iter().any(|s| s.name == *owner) {
                                let arg_strs: Vec<String> =
                                    args.iter().map(|a| self.materialize(a)).collect();
                                let arg_c = arg_strs.join(", ");
                                let has_managed = {
                                    let mut m = false;
                                    for a in args {
                                        if let Some(ty) = self.type_of(a).cloned()
                                            && self.tcx.root_slot_count(&ty) > 0
                                        {
                                            m = true;
                                            break;
                                        }
                                    }
                                    m
                                };
                                if self.hoist && has_managed {
                                    let scope_var = self.fresh_id("gc_scope");
                                    let _ = writeln!(
                                        self.pre,
                                        "    size_t {scope_var} = astra_gc_scope_enter();"
                                    );
                                    for (arg, arg_str) in args.iter().zip(arg_strs.iter()) {
                                        if let Some(ty) = self.type_of(arg).cloned() {
                                            for slot in self.tcx.root_exprs(arg_str, &ty) {
                                                let _ = writeln!(
                                                    self.pre,
                                                    "    astra_gc_root({scope_var}, {slot});"
                                                );
                                            }
                                        }
                                    }
                                    let call_ty = self.type_of(e).cloned().unwrap_or(Type::Unit);
                                    if matches!(call_ty, Type::Unit) {
                                        let _ = writeln!(
                                            self.pre,
                                            "    {}({arg_c});",
                                            sanitize(member)
                                        );
                                        let _ = writeln!(
                                            self.pre,
                                            "    astra_gc_scope_exit({scope_var});"
                                        );
                                        return "(NS_Unit){0}".to_string();
                                    }
                                    let call_c_ty = self.tcx.c_type(&call_ty);
                                    let result_tmp = self.fresh_id("script_ret");
                                    let _ = writeln!(
                                        self.pre,
                                        "    {call_c_ty} {result_tmp} = {}({arg_c});",
                                        sanitize(member)
                                    );
                                    let _ =
                                        writeln!(self.pre, "    astra_gc_scope_exit({scope_var});");
                                    if self.tcx.root_slot_count(&call_ty) > 0 {
                                        self.publish_temp_in_pre(&result_tmp, &call_ty);
                                    }
                                    return result_tmp;
                                }
                                return format!("{}({arg_c})", sanitize(member));
                            }
                        }
                    }
                }

                self.method_c(target, member, args, e)
            }
            _ => {
                self.tcx.rejects.push(reject(
                    "NS8017",
                    "calling this callee is not supported by the C backend first slice",
                ));
                "(NS_Unit){0}".into()
            }
        }
    }

    fn materialize(&mut self, e: &Expr) -> String {
        if !self.hoist {
            return self.expr(e);
        }
        let Some(ty) = self.type_of(e).cloned() else {
            return self.expr(e);
        };
        if self.tcx.root_slot_count(&ty) == 0 {
            let side_effecting = matches!(
                e.kind,
                ExprKind::Call { .. }
                    | ExprKind::Block(_)
                    | ExprKind::If { .. }
                    | ExprKind::Match { .. }
            );
            if !side_effecting || self.root_base_name(e).is_some() {
                return self.expr(e);
            }
        } else if self.root_base_name(e).is_some() {
            return self.expr(e);
        }
        let ty_c = self.tcx.c_type(&ty);
        let tmp = self.freshid("arg");
        let value = self.expr(e);
        let _ = writeln!(self.pre, "    {ty_c} {tmp} = {value};");
        self.publish_temp_in_pre(&tmp, &ty);
        tmp
    }

    fn publish_temp_in_pre(&mut self, name: &str, ty: &Type) {
        let Some(var) = self.roots.as_ref().map(|frame| frame.var.clone()) else {
            return;
        };
        let first = self.roots.as_ref().map_or(0, |frame| frame.used);
        let slots = self.tcx.root_exprs(name, ty);
        for (offset, slot) in slots.iter().enumerate() {
            let _ = writeln!(
                self.pre,
                "    astra_gc_publish({var} + {}, {slot});",
                first + offset
            );
        }
        if let Some(frame) = self.roots.as_mut() {
            frame.used += slots.len();
        }
    }

    fn args_c(&mut self, args: &[Expr]) -> String {
        args.iter()
            .map(|a| self.materialize(a))
            .collect::<Vec<_>>()
            .join(", ")
    }

    fn some_c(&mut self, args: &[Expr], e: &Expr) -> String {
        let ty = self
            .type_of(e)
            .cloned()
            .unwrap_or(Type::Option(Box::new(Type::I32)));
        let inner = match &ty {
            Type::Option(t) => (**t).clone(),
            _ => Type::I32,
        };
        let opt_c = self.tcx.c_type(&ty);
        let val = args
            .first()
            .map(|a| self.expr(a))
            .unwrap_or_else(|| "(NS_Unit){0}".into());
        let _ = self.tcx.c_type(&inner);
        format!("({opt_c}){{ .some = true, .value = {val} }}")
    }

    fn ok_c(&mut self, args: &[Expr], e: &Expr) -> String {
        let ty = self
            .type_of(e)
            .cloned()
            .unwrap_or(Type::Result(Box::new(Type::Unit), Box::new(Type::Unit)));
        let res_c = self.tcx.c_type(&ty);
        let val = args
            .first()
            .map(|a| self.expr(a))
            .unwrap_or_else(|| "(NS_Unit){0}".into());
        format!("({res_c}){{ .ok = true, .value = {val} }}")
    }

    fn err_c(&mut self, args: &[Expr], e: &Expr) -> String {
        let ty = self
            .type_of(e)
            .cloned()
            .unwrap_or(Type::Result(Box::new(Type::Unit), Box::new(Type::Unit)));
        let res_c = self.tcx.c_type(&ty);
        let val = args
            .first()
            .map(|a| self.expr(a))
            .unwrap_or_else(|| "(NS_Unit){0}".into());
        format!("({res_c}){{ .ok = false, .error = {val} }}")
    }

    fn enum_variant_c(&mut self, owner: &str, variant: &str, args: &[Expr]) -> String {
        let enum_c = self.tcx.c_type(&Type::Named(owner.to_string(), Vec::new()));

        let Some(tag) = self.tcx.enum_tag(owner, variant) else {
            self.tcx
                .rejects
                .push(reject("NS8020", format!("enum '{owner}' is not declared")));
            return format!("({enum_c}){{0}}");
        };
        let fields: Vec<Type> = match self.tcx.enum_shape(owner) {
            Some(shape) => shape
                .variants
                .iter()
                .find(|v| v.name.as_str() == variant)
                .map(|v| v.fields.clone())
                .unwrap_or_default(),
            None => {
                self.tcx
                    .rejects
                    .push(reject("NS8020", format!("enum '{owner}' is not declared")));
                return format!("({enum_c}){{0}}");
            }
        };
        if fields.is_empty() && args.is_empty() {
            return format!("({enum_c}){{ .tag = {tag} }}");
        }
        if fields.is_empty() {
            self.tcx.rejects.push(reject(
                "NS8020",
                format!("enum variant '{owner}::{variant}' takes no arguments"),
            ));
            return format!("({enum_c}){{ .tag = {tag} }}");
        }
        let member = field_c(variant);
        let arg_vals: Vec<String> = args.iter().map(|a| self.expr(a)).collect();
        if fields.len() == 1 {
            format!(
                "({enum_c}){{ .tag = {tag}, .payload.{member} = {val} }}",
                val = arg_vals[0]
            )
        } else {
            let joined = arg_vals.join(", ");
            format!("({enum_c}){{ .tag = {tag}, .payload.{member} = {{ {joined} }} }}")
        }
    }

    fn method_c(&mut self, receiver: &Expr, name: &str, args: &[Expr], call: &Expr) -> String {
        let rt = self.type_of(receiver).cloned();

        if let Some(ty) = &rt
            && ty.is_integer()
            && let Some(result) = self.integer_method_c(ty, name, receiver, args)
        {
            return result;
        }
        match (&rt, name) {
            (Some(Type::Named(type_name, _)), "send") if type_name == "Channel" => {
                let receiver_c = self.expr(receiver);
                let value = args
                    .first()
                    .map(|a| self.expr(a))
                    .unwrap_or_else(|| "(NS_Unit){0}".into());
                let value_ty = args
                    .first()
                    .and_then(|a| self.type_of(a).cloned())
                    .unwrap_or(Type::Unit);
                let value_c = self.tcx.c_type(&value_ty);
                let result_ty = Type::Result(
                    Box::new(Type::Unit),
                    Box::new(Type::Named("ChannelError".to_owned(), Vec::new())),
                );
                let result_c = self.tcx.c_type(&result_ty);
                let channel = self.fresh_id("chan");
                let value_tmp = self.fresh_id("cval");
                let result = self.fresh_id("cres");
                let _ = writeln!(self.pre, "    NS_Channel* {channel} = {receiver_c};");
                let _ = writeln!(self.pre, "    {value_c} {value_tmp} = {value};");
                let _ = writeln!(
                    self.pre,
                    "    {result_c} {result} = astra_channel_send({channel}, &{value_tmp}) ? ({result_c}){{ .ok = true, .value = (NS_Unit){{0}} }} : ({result_c}){{ .ok = false, .error = (NS_ChannelError){{0}} }};"
                );
                result
            }
            (Some(Type::Named(type_name, arguments)), "receive") if type_name == "Channel" => {
                let element = arguments.first().cloned().unwrap_or(Type::Unit);
                let element_c = self.tcx.c_type(&element);
                let zero = self.zero(&element);
                let result_ty = Type::Result(
                    Box::new(element),
                    Box::new(Type::Named("ChannelError".to_owned(), Vec::new())),
                );
                let result_c = self.tcx.c_type(&result_ty);
                let receiver_c = self.expr(receiver);
                let channel = self.fresh_id("chan");
                let value = self.fresh_id("cval");
                let result = self.fresh_id("cres");
                let _ = writeln!(self.pre, "    NS_Channel* {channel} = {receiver_c};");
                let _ = writeln!(self.pre, "    {element_c} {value} = {zero};");
                let _ = writeln!(self.pre, "    {result_c} {result} = ({result_c}){{0}};");
                let _ = writeln!(
                    self.pre,
                    "    if (astra_channel_receive({channel}, &{value})) {{ {result}.ok = true; {result}.value = {value}; }} else {{ {result}.error = (NS_ChannelError){{0}}; }}"
                );
                result
            }
            (Some(Type::Named(type_name, _)), "close") if type_name == "Channel" => {
                let receiver_c = self.expr(receiver);
                let _ = writeln!(self.pre, "    astra_channel_close({receiver_c});");
                "(NS_Unit){0}".into()
            }
            (Some(Type::Named(type_name, _)), "join") if type_name == "Task" => {
                let receiver_c = self.expr(receiver);
                let task_ty = self.type_of(receiver).cloned().unwrap_or(Type::Unit);
                let task_c = self.tcx.c_type(&task_ty);
                let task = self.fresh_id("task");
                let _ = writeln!(self.pre, "    {task_c} {task} = {receiver_c};");
                let pending = PendingTask {
                    var: task.clone(),
                    task_ty,
                };
                let code = self.task_join_code(&pending);
                self.pre.push_str(&code);
                format!("({task}->value)")
            }
            (Some(Type::Named(type_name, _)), "cancel") if type_name == "Task" => {
                let receiver_c = self.expr(receiver);
                let task_ty = self.type_of(receiver).cloned().unwrap_or(Type::Unit);
                let task_c = self.tcx.c_type(&task_ty);
                let task = self.fresh_id("task");
                let _ = writeln!(self.pre, "    {task_c} {task} = {receiver_c};");
                let _ = writeln!(self.pre, "    astra_task_cancel(&{task}->core);");
                "(NS_Unit){0}".into()
            }
            (Some(Type::Named(type_name, _)), "canceled") if type_name == "Task" => {
                let receiver_c = self.expr(receiver);
                let task_ty = self.type_of(receiver).cloned().unwrap_or(Type::Unit);
                let task_c = self.tcx.c_type(&task_ty);
                let task = self.fresh_id("task");
                let _ = writeln!(self.pre, "    {task_c} {task} = {receiver_c};");
                format!("({task}->core.canceled)")
            }

            (Some(Type::Named(type_name, _)), "read") if type_name == "Ptr" => {
                let receiver_c = self.expr(receiver);
                let index = args
                    .first()
                    .map(|a| self.expr(a))
                    .unwrap_or_else(|| "0".into());
                format!("({receiver_c})[{index}]")
            }
            (Some(Type::Named(type_name, _)), "write") if type_name == "Ptr" => {
                let receiver_c = self.expr(receiver);
                let index = args
                    .first()
                    .map(|a| self.expr(a))
                    .unwrap_or_else(|| "0".into());
                let value = args
                    .get(1)
                    .map(|a| self.expr(a))
                    .unwrap_or_else(|| "0".into());
                let _ = writeln!(self.pre, "    {receiver_c}[{index}] = {value};");
                "(NS_Unit){0}".into()
            }

            (Some(Type::Named(type_name, _)), "alloc") if type_name == "Region" => {
                let receiver_c = self.expr(receiver);
                let ptr_ty = self
                    .type_of(call)
                    .cloned()
                    .unwrap_or_else(|| Type::Named("Ptr".to_owned(), vec![Type::U8]));
                let element = match &ptr_ty {
                    Type::Named(_, arguments) => arguments.first().cloned().unwrap_or(Type::U8),
                    _ => Type::U8,
                };
                let elem_c = self.tcx.c_type(&element);
                let count = args
                    .first()
                    .map(|a| self.expr(a))
                    .unwrap_or_else(|| "0".to_owned());
                format!(
                    "(({elem_c}*)astra_region_alloc({receiver_c}, (size_t)({count}) * sizeof({elem_c})))"
                )
            }
            (Some(Type::Named(type_name, _)), "free") if type_name == "Region" => {
                let receiver_c = self.expr(receiver);
                let _ = writeln!(self.pre, "    astra_region_free({receiver_c});");
                "(NS_Unit){0}".into()
            }
            (Some(Type::Str), "len") => {
                let r = self.expr(receiver);
                format!("({r}).len")
            }
            (Some(Type::Str), "is_empty") => {
                let r = self.expr(receiver);
                format!("(({r}).len == 0)")
            }
            (Some(Type::Array(_) | Type::Slice(_)), "len") => {
                let r = self.expr(receiver);
                format!("({r}).len")
            }
            (Some(Type::Array(_) | Type::Slice(_)), "push") => {
                let r = self.expr(receiver);
                let elem_ty = self.elem_type_of(receiver);
                let elem_c = self.tcx.c_type(&elem_ty);
                let shape = self.tcx.block_shape_ref(&elem_ty);
                let val = args
                    .first()
                    .map(|a| self.expr(a))
                    .unwrap_or_else(|| "0".into());
                format!(
                    "astra_array_push(&({r}), sizeof({elem_c}), &({elem_c}){{ {val} }}, {shape})"
                )
            }
            (Some(Type::Array(_) | Type::Slice(_)), "get") => {
                let elem_ty = self.elem_type_of(receiver);
                let elem_c = self.tcx.c_type(&elem_ty);
                let arr_ty = self
                    .type_of(receiver)
                    .cloned()
                    .unwrap_or(Type::Array(Box::new(elem_ty.clone())));
                let arr_c = self.tcx.c_type(&arr_ty);
                let opt_ty = Type::Option(Box::new(elem_ty));
                let opt_c = self.tcx.c_type(&opt_ty);
                let arr = self.fresh_id("arr");
                let idx = self.fresh_id("idx");
                let left = self.expr(receiver);
                let right = args
                    .first()
                    .map(|a| self.expr(a))
                    .unwrap_or_else(|| "0".into());
                let _ = writeln!(self.pre, "    {arr_c} {arr} = {left};");
                let _ = writeln!(self.pre, "    size_t {idx} = (size_t)({right});");
                format!(
                    "({idx} < ({arr}).len ? ({opt_c}){{ .some = true, .value = *({elem_c}*)astra_array_index(&({arr}), {idx}, sizeof({elem_c})) }} : ({opt_c}){{0}})"
                )
            }
            (Some(Type::Option(_)), "is_some") => {
                let r = self.expr(receiver);
                format!("({r}).some")
            }
            (Some(Type::Option(_)), "is_none") => {
                let r = self.expr(receiver);
                format!("(!({r}).some)")
            }
            (Some(Type::Option(inner)), "unwrap") => {
                let r = self.expr(receiver);
                let opt_c = self.tcx.c_type(&Type::Option(inner.clone()));
                let tmp = self.fresh_id("optsome");
                let _ = writeln!(self.pre, "    {opt_c} {tmp} = {r};");
                let _ = writeln!(
                    self.pre,
                    "    if (!{tmp}.some) astra_panic(\"called unwrap() on None\");"
                );
                format!("{tmp}.value")
            }
            (Some(Type::Option(inner)), "unwrap_or") => {
                let r = self.expr(receiver);
                let fallback = args
                    .first()
                    .map(|a| self.expr(a))
                    .unwrap_or_else(|| "0".into());
                let opt_c = self.tcx.c_type(&Type::Option(inner.clone()));
                let tmp = self.fresh_id("optor");
                let _ = writeln!(self.pre, "    {opt_c} {tmp} = {r};");

                format!("({tmp}.some ? {tmp}.value : ({fallback}))")
            }
            (Some(Type::Result(ok, err)), "is_ok") => {
                let r = self.expr(receiver);
                let _ = (ok, err);
                format!("({r}).ok")
            }
            (Some(Type::Result(_, _)), "is_err") => {
                let r = self.expr(receiver);
                format!("(!({r}).ok)")
            }
            (Some(Type::Result(ok, err)), "unwrap") => {
                let r = self.expr(receiver);
                let res_c = self.tcx.c_type(&Type::Result(ok.clone(), err.clone()));
                let tmp = self.fresh_id("resok");
                let _ = writeln!(self.pre, "    {res_c} {tmp} = {r};");
                let _ = writeln!(
                    self.pre,
                    "    if (!{tmp}.ok) astra_panic(\"called unwrap() on Err\");"
                );
                format!("{tmp}.value")
            }
            (Some(Type::Result(ok, err)), "unwrap_or") => {
                let r = self.expr(receiver);
                let fallback = args
                    .first()
                    .map(|a| self.expr(a))
                    .unwrap_or_else(|| "0".into());
                let res_c = self.tcx.c_type(&Type::Result(ok.clone(), err.clone()));
                let tmp = self.fresh_id("resor");
                let _ = writeln!(self.pre, "    {res_c} {tmp} = {r};");

                format!("({tmp}.ok ? {tmp}.value : ({fallback}))")
            }
            (Some(Type::Result(ok, err)), "error") => {
                let r = self.expr(receiver);
                let res_c = self.tcx.c_type(&Type::Result(ok.clone(), err.clone()));
                let tmp = self.fresh_id("reserr");
                let _ = writeln!(self.pre, "    {res_c} {tmp} = {r};");
                let _ = writeln!(
                    self.pre,
                    "    if ({tmp}.ok) astra_panic(\"called error() on Ok\");"
                );
                format!("{tmp}.error")
            }
            (Some(Type::Str), "slice") => {
                let r = self.expr(receiver);
                let start = args
                    .first()
                    .map(|a| self.expr(a))
                    .unwrap_or_else(|| "0".into());
                let end = args
                    .get(1)
                    .map(|a| self.expr(a))
                    .unwrap_or_else(|| "0".into());
                let res_ty = Type::Result(
                    Box::new(Type::Str),
                    Box::new(Type::Named("SliceError".to_owned(), Vec::new())),
                );
                let res_c = self.tcx.c_type(&res_ty);
                let part = self.fresh_id("slicepart");
                let ok = self.fresh_id("slicek");
                let res = self.fresh_id("sliceres");
                let _ = writeln!(self.pre, "    NS_Str {part};");
                let _ = writeln!(
                    self.pre,
                    "    bool {ok} = astra_str_slice_try({r}, (size_t)({start}), (size_t)({end}), &{part});"
                );
                let _ = writeln!(
                    self.pre,
                    "    {res_c} {res} = {ok} ? ({res_c}){{ .ok = true, .value = {part} }} : ({res_c}){{ .ok = false, .error = (NS_SliceError){{ \"slice range is out of bounds or splits a UTF-8 scalar value\" }} }};"
                );
                res
            }
            (Some(Type::Str), "to_int") => {
                let r = self.expr(receiver);
                let source = self.fresh_id("intsrc");
                let value = self.fresh_id("intval");
                let ok = self.fresh_id("intok");
                let res = self.fresh_id("intres");
                let res_ty = Type::Result(
                    Box::new(Type::I32),
                    Box::new(Type::Named("ParseIntError".to_owned(), Vec::new())),
                );
                let res_c = self.tcx.c_type(&res_ty);
                let _ = writeln!(self.pre, "    NS_Str {source} = {r};");
                let _ = writeln!(self.pre, "    int32_t {value} = 0;");
                let _ = writeln!(
                    self.pre,
                    "    bool {ok} = astra_str_to_int_try({source}, &{value});"
                );
                let _ = writeln!(
                    self.pre,
                    "    {res_c} {res} = {ok} ? ({res_c}){{ .ok = true, .value = {value} }} : ({res_c}){{ .ok = false, .error = (NS_ParseIntError){{ {source} }} }};"
                );
                res
            }
            (Some(Type::Str), "byte_at") => {
                let r = self.expr(receiver);
                let index = args
                    .first()
                    .map(|a| self.expr(a))
                    .unwrap_or_else(|| "0".into());
                let opt_c = self.tcx.c_type(&Type::Option(Box::new(Type::U8)));
                let source = self.fresh_id("bytesrc");
                let at = self.fresh_id("byteidx");
                let res = self.fresh_id("byteopt");
                let _ = writeln!(self.pre, "    NS_Str {source} = {r};");
                let _ = writeln!(self.pre, "    size_t {at} = (size_t)({index});");
                let _ = writeln!(
                    self.pre,
                    "    {opt_c} {res} = {at} < {source}.len ? ({opt_c}){{ .some = true, .value = (uint8_t){source}.data[{at}] }} : ({opt_c}){{ .some = false }};"
                );
                res
            }
            (Some(Type::Str), "char_at") => {
                let r = self.expr(receiver);
                let index = args
                    .first()
                    .map(|a| self.expr(a))
                    .unwrap_or_else(|| "0".into());
                let opt_c = self.tcx.c_type(&Type::Option(Box::new(Type::Str)));
                let source = self.fresh_id("charsrc");
                let at = self.fresh_id("charidx");
                let part = self.fresh_id("charpart");
                let ok = self.fresh_id("charok");
                let res = self.fresh_id("charopt");
                let _ = writeln!(self.pre, "    NS_Str {source} = {r};");
                let _ = writeln!(self.pre, "    size_t {at} = (size_t)({index});");
                let _ = writeln!(self.pre, "    NS_Str {part};");
                let _ = writeln!(
                    self.pre,
                    "    bool {ok} = astra_str_char_at({source}, {at}, &{part});"
                );
                let _ = writeln!(
                    self.pre,
                    "    {opt_c} {res} = {ok} ? ({opt_c}){{ .some = true, .value = {part} }} : ({opt_c}){{ .some = false }};"
                );
                res
            }
            (Some(Type::Str), "char_count") => {
                let r = self.expr(receiver);
                format!("astra_str_char_count({r})")
            }
            (Some(Type::Str), "bytes") => {
                let r = self.expr(receiver);
                format!("astra_str_bytes({r})")
            }
            (Some(Type::Str), "chars") => {
                let r = self.expr(receiver);
                format!("astra_str_chars({r})")
            }
            (None, "new") if matches!(&receiver.kind, ExprKind::Identifier(name) if name == "bytes") =>
            {
                let length = args
                    .first()
                    .map(|a| self.expr(a))
                    .unwrap_or_else(|| "0".into());
                format!("astra_bytes_new((size_t)({length}))")
            }
            (Some(Type::Bytes), "len") => {
                let r = self.expr(receiver);
                format!("({r}).len")
            }
            (Some(Type::Bytes), "is_empty") => {
                let r = self.expr(receiver);
                format!("(({r}).len == 0)")
            }
            (Some(Type::Bytes), "slice") => {
                let r = self.expr(receiver);
                let start = args
                    .first()
                    .map(|a| self.expr(a))
                    .unwrap_or_else(|| "0".into());
                let end = args
                    .get(1)
                    .map(|a| self.expr(a))
                    .unwrap_or_else(|| "0".into());
                format!("astra_bytes_slice({r}, (size_t)({start}), (size_t)({end}))")
            }
            (Some(Type::Map(key_ty, _)), "len") => {
                let _ = key_ty;
                let r = self.expr(receiver);
                format!("astra_map_len(&({r}))")
            }
            (Some(Type::Map(key_ty, val_ty)), "contains") => {
                let kt = (**key_ty).clone();
                let key_c = self.tcx.c_type(&kt);
                let r = self.expr(receiver);
                let k = args
                    .first()
                    .map(|a| self.expr(a))
                    .unwrap_or_else(|| "0".into());
                let _ = val_ty;

                let ktmp = self.freshid("mk");
                let _ = writeln!(self.pre, "    {key_c} {ktmp} = {k};");
                format!("astra_map_contains(&({r}), &{ktmp})")
            }
            (Some(Type::Map(key_ty, val_ty)), "get") => {
                let kt = (**key_ty).clone();
                let vt = (**val_ty).clone();
                let key_c = self.tcx.c_type(&kt);
                let val_c = self.tcx.c_type(&vt);
                let r = self.expr(receiver);
                let k = args
                    .first()
                    .map(|a| self.expr(a))
                    .unwrap_or_else(|| "0".into());

                let ktmp = self.freshid("mk");
                let _ = writeln!(self.pre, "    {key_c} {ktmp} = {k};");

                let opt_c = self.tcx.c_type(&Type::Option(Box::new(vt.clone())));
                let tmp = self.freshid("mg");
                let _ = writeln!(
                    self.pre,
                    "    void* {tmp}_ptr = astra_map_get(&({r}), &{ktmp});"
                );
                let _ = writeln!(
                    self.pre,
                    "    {opt_c} {tmp} = {tmp}_ptr ? ({opt_c}){{ .some = true, .value = *({val_c}*){tmp}_ptr }} : ({opt_c}){{ .some = false }};"
                );
                tmp
            }
            (Some(Type::Map(key_ty, val_ty)), "insert") => {
                let kt = (**key_ty).clone();
                let vt = (**val_ty).clone();
                let key_c = self.tcx.c_type(&kt);
                let val_c = self.tcx.c_type(&vt);
                let r = self.expr(receiver);
                let k = args
                    .first()
                    .map(|a| self.expr(a))
                    .unwrap_or_else(|| "0".into());
                let v = args
                    .get(1)
                    .map(|a| self.expr(a))
                    .unwrap_or_else(|| "0".into());
                let mtmp = self.freshid("mr");
                let _ = writeln!(self.pre, "    NS_Map* {mtmp} = &({r});");
                let ktmp = self.freshid("mk");
                let vtmp = self.freshid("mv");
                let _ = writeln!(self.pre, "    {key_c} {ktmp} = {k};");
                let _ = writeln!(self.pre, "    {val_c} {vtmp} = {v};");
                let opt_c = self.tcx.c_type(&Type::Option(Box::new(vt.clone())));
                let tmp = self.freshid("mi");
                let _ = writeln!(
                    self.pre,
                    "    void* {tmp}_prev = astra_map_get({mtmp}, &{ktmp});"
                );
                let _ = writeln!(
                    self.pre,
                    "    {opt_c} {tmp} = {tmp}_prev ? ({opt_c}){{ .some = true, .value = *({val_c}*){tmp}_prev }} : ({opt_c}){{ .some = false }};"
                );
                let _ = writeln!(self.pre, "    astra_map_set({mtmp}, &{ktmp}, &{vtmp});");
                tmp
            }
            _ => {
                self.tcx.rejects.push(reject(
                    "NS8021",
                    format!("method '.{name}()' is not supported by the C backend first slice"),
                ));
                "0".into()
            }
        }
    }

    fn print_c(&mut self, args: &[Expr], nl: bool) -> String {
        let Some(arg) = args.first() else {
            return if nl {
                "astra_println_unit()".into()
            } else {
                "astra_print_unit()".into()
            };
        };
        let ty = self.type_of(arg).cloned().unwrap_or(Type::Unit);
        let val = self.expr(arg);
        let suffix = match &ty {
            Type::I8 => "i8",
            Type::I16 => "i16",
            Type::I32 => "i32",
            Type::I64 => "i64",
            Type::U8 => "u8",
            Type::U16 => "u16",
            Type::U32 => "u32",
            Type::U64 => "u64",
            Type::Usize => "usize",
            Type::Isize => "isize",
            Type::F32 => "f32",
            Type::F64 => "f64",
            Type::Bool => "bool",
            Type::Str => "str",
            Type::Bytes => "bytes",
            Type::Unit => "unit",
            _ => {
                self.tcx.rejects.push(reject(
                    "NS8024",
                    "printing this value type is not supported by the C backend first slice",
                ));
                return "0".into();
            }
        };
        let kind = if nl { "println" } else { "print" };
        format!("astra_{kind}_{suffix}({val})")
    }

    fn assert_c(&mut self, args: &[Expr]) -> String {
        let val = args
            .first()
            .map(|a| self.expr(a))
            .unwrap_or_else(|| "false".into());
        format!("astra_assert({val})")
    }

    fn index_c(&mut self, target: &Expr, index: &Expr, _e: &Expr) -> String {
        let tt = self.type_of(target).cloned();
        let t = self.expr(target);
        let i = self.expr(index);
        if matches!(tt, Some(Type::Bytes)) {
            return format!("astra_bytes_get({t}, {i})");
        }
        if let Some(Type::Map(key_ty, val_ty)) = &tt {
            let map_c = self.tcx.c_type(tt.as_ref().expect("map type"));
            let key_c = self.tcx.c_type(key_ty);
            let val_c = self.tcx.c_type(val_ty);
            let mtmp = self.freshid("mx");
            let ktmp = self.freshid("mk");
            let ptr = self.freshid("mv");
            let _ = writeln!(self.pre, "    {map_c} {mtmp} = {t};");
            let _ = writeln!(self.pre, "    {key_c} {ktmp} = {i};");
            let _ = writeln!(
                self.pre,
                "    void* {ptr} = astra_map_get(&{mtmp}, &{ktmp});"
            );
            let _ = writeln!(
                self.pre,
                "    if ({ptr} == NULL) {{ astra_panic(\"map key is not present\"); }}"
            );
            return format!("(*({val_c}*){ptr})");
        }
        let elem_ty = self.elem_type_of(target);
        let elem_c = self.tcx.c_type(&elem_ty);
        format!("(*({elem_c}*)astra_array_index(&({t}), {i}, sizeof({elem_c})))")
    }

    fn member_c(&mut self, target: &Expr, member: &str, e: &Expr) -> String {
        if let ExprKind::Identifier(owner) = &target.kind
            && let Some(shape) = self.tcx.enum_shape(owner)
            && shape.variants.iter().any(|v| v.name == member)
        {
            return self.enum_variant_c(owner, member, &[]);
        }
        let tt = self.type_of(target).cloned();
        let t = self.expr(target);
        match &tt {
            Some(Type::Named(name, _)) if self.tcx.struct_shape(name).is_some() => {
                let shape = self.tcx.struct_shape(name);
                if let Some(s) = &shape
                    && s.fields.iter().any(|(f, _)| f == member)
                {
                    return format!("({t}).{m}", m = field_c(member));
                }
                self.tcx.rejects.push(reject(
                    "NS8020",
                    format!("struct '{name}' has no field '{member}'"),
                ));
                format!("({t}).{m}", m = field_c(member))
            }
            Some(Type::Tuple(_)) => {
                if let Ok(idx) = member.parse::<usize>() {
                    return format!("({t})._{idx}");
                }
                self.tcx.rejects.push(reject(
                    "NS8021",
                    "tuple field access is not supported by the C backend first slice",
                ));
                "0".into()
            }
            _ => {
                let _ = e;
                self.tcx.rejects.push(reject(
                    "NS8021",
                    format!(
                        "field access '.{member}' is not supported by the C backend first slice"
                    ),
                ));
                "0".into()
            }
        }
    }

    fn array_literal_c(&mut self, elements: &[Expr], _e: &Expr) -> String {
        let elem_ty = elements
            .first()
            .and_then(|el| self.type_of(el).cloned())
            .unwrap_or(Type::I32);
        let elem_c = self.tcx.c_type(&elem_ty);
        let shape = self.tcx.block_shape_ref(&elem_ty);
        let vals: Vec<String> = elements.iter().map(|el| self.materialize(el)).collect();
        let n = vals.len();
        let joined = vals.join(", ");
        format!(
            "astra_array_new_lit({n}, sizeof({elem_c}), (const {elem_c}[]){{ {joined} }}, {shape})"
        )
    }

    fn map_literal_c(&mut self, entries: &[ast::MapEntry], e: &Expr) -> String {
        let ty = self
            .type_of(e)
            .cloned()
            .unwrap_or(Type::Map(Box::new(Type::Str), Box::new(Type::I32)));
        let (key_ty, val_ty) = match &ty {
            Type::Map(k, v) => ((**k).clone(), (**v).clone()),
            _ => (Type::Str, Type::I32),
        };
        let key_c = self.tcx.c_type(&key_ty);
        let val_c = self.tcx.c_type(&val_ty);
        let key_shape = self.tcx.block_shape_ref(&key_ty);
        let val_shape = self.tcx.block_shape_ref(&val_ty);
        let key_is_str = key_ty == Type::Str;
        let tmp = self.freshid("map");
        let _ = writeln!(
            self.pre,
            "    NS_Map {tmp} = astra_map_new(sizeof({key_c}), sizeof({val_c}), {key_is_str}, {key_shape}, {val_shape});"
        );

        self.publish_temp_in_pre(&tmp, &ty);
        for entry in entries {
            let k = self.materialize(&entry.key);
            let v = self.materialize(&entry.value);
            let ktmp = self.freshid("mk");
            let vtmp = self.freshid("mv");
            let _ = writeln!(self.pre, "    {key_c} {ktmp} = {k};");
            let _ = writeln!(self.pre, "    {val_c} {vtmp} = {v};");
            let _ = writeln!(self.pre, "    astra_map_set(&{tmp}, &{ktmp}, &{vtmp});");
        }
        tmp
    }

    fn c_bytes_literal(&mut self, bytes: &[u8]) -> String {
        let len = bytes.len();
        if len == 0 {
            return "(NS_Bytes){ .data = (const uint8_t*)\"\", .len = 0 }".into();
        }
        let hex: String = bytes.iter().map(|b| format!("0x{b:02X},")).collect();
        format!("(NS_Bytes){{ .data = (const uint8_t[]){{ {hex} }}, .len = {len} }}")
    }

    fn tuple_c(&mut self, elements: &[Expr], _e: &Expr) -> String {
        let ty = self
            .type_of(&Expr {
                kind: ExprKind::Tuple(elements.to_vec()),
                span: astra_core::span::Span::default(),
            })
            .cloned()
            .unwrap_or(Type::Unit);
        if !matches!(ty, Type::Tuple(_)) {
            self.tcx.rejects.push(reject(
                "NS8025",
                "tuple type is not available for this tuple expression",
            ));
            return "(NS_Unit){0}".into();
        }
        let tuple_c = self.tcx.c_type(&ty);
        let vals: Vec<String> = elements.iter().map(|el| self.materialize(el)).collect();
        let joined = vals.join(", ");
        format!("({tuple_c}){{ {joined} }}")
    }

    fn struct_init_c(&mut self, path: &ast::TypePath, fields: &[ast::FieldInit]) -> String {
        let name = path.segments.join(".");
        let ty_c = self.tcx.c_type(&Type::Named(name.clone(), Vec::new()));
        let Some(shape) = self.tcx.struct_shape(&name) else {
            self.tcx
                .rejects
                .push(reject("NS8020", format!("struct '{name}' is not declared")));
            return format!("({ty_c}){{0}}");
        };
        if shape.tuple {
            let vals: Vec<String> = fields.iter().map(|f| self.materialize(&f.value)).collect();
            let joined = vals.join(", ");
            format!("({ty_c}){{ {joined} }}")
        } else {
            let mut inits = String::new();
            for f in fields {
                let val = self.materialize(&f.value);
                let _ = write!(inits, ".{} = {}, ", field_c(&f.name), val);
            }
            format!("({ty_c}){{ {inits}}}")
        }
    }

    fn spawn_c(&mut self, body: &ast::Block, spawn_expr: &Expr) -> String {
        let task_ty = self
            .type_of(spawn_expr)
            .cloned()
            .unwrap_or_else(|| Type::Named("Task".to_owned(), vec![Type::Unit]));
        let inner = task_value_type(&task_ty);
        let task_c = self.tcx.c_type(&task_ty);
        let task_struct = self.tcx.task_struct_ref(&inner);
        let task_block_shape = self.tcx.task_block_shape_ref(&inner);
        let inner_c = self.tcx.c_type(&inner);
        let inner_shape = self.tcx.value_shape_ref(&inner);

        let captures = self.spawn_captures(body);
        let id = self.fresh_id("spawn");
        let closure_c = format!("ns_closure_{id}");
        let body_sym = format!("ns_spawn_body_{id}");
        let entry_sym = format!("ns_spawn_entry_{id}");

        let mut fields = String::new();
        for (name, ty) in &captures {
            let ty_c = self.tcx.c_type(ty);
            let _ = writeln!(fields, "    {ty_c} c_{};", sanitize(name));
        }
        let _ = writeln!(fields, "    {inner_c} value;");
        self.tcx.ensure(
            &closure_c,
            format!("typedef struct {{\n{fields}}} {closure_c};"),
        );

        let mut shape_fields: Vec<String> = Vec::new();
        for (name, ty) in &captures {
            let shape = self.tcx.value_shape_ref(ty);
            if shape != "&NS_SHAPE_LEAF" {
                shape_fields.push(format!(
                    "{{ offsetof({closure_c}, c_{}), {shape} }}",
                    sanitize(name)
                ));
            }
        }
        if inner_shape != "&NS_SHAPE_LEAF" {
            shape_fields.push(format!("{{ offsetof({closure_c}, value), {inner_shape} }}"));
        }
        let closure_shape = if shape_fields.is_empty() {
            "&NS_SHAPE_LEAF".to_owned()
        } else {
            self.tcx
                .record_shape_ref(&shape_fields.join(", "), shape_fields.len())
        };

        self.task_protos
            .push(format!("static {inner_c} {body_sym}({closure_c}* _c);"));
        self.task_protos
            .push(format!("static void {entry_sym}(void* _arg);"));
        self.emit_spawn_body(&body_sym, &entry_sym, &closure_c, &inner, &captures, body);

        let task_var = self.fresh_id("task");
        let closure_var = self.fresh_id("closure");
        let _ = writeln!(
            self.pre,
            "    {task_c} {task_var} = ({task_c})astra_gc_alloc({task_block_shape}, sizeof({task_struct}));"
        );
        self.publish_temp_in_pre(&task_var, &task_ty);
        let _ = writeln!(self.pre, "    {task_var}->core.heap = astra_heap_create();");
        let _ = writeln!(
            self.pre,
            "    {closure_c}* {closure_var} = ({closure_c}*)astra_gc_alloc_in_heap({task_var}->core.heap, {closure_shape}, sizeof({closure_c}));"
        );
        let _ = writeln!(
            self.pre,
            "    astra_gc_root_in({task_var}->core.heap, {closure_var});"
        );
        for (name, ty) in &captures {
            let field = format!("{closure_var}->c_{}", sanitize(name));
            let local = format!("v_{}", sanitize(name));
            if is_shared_capture(ty) {
                let _ = writeln!(self.pre, "    {field} = {local};");
                continue;
            }
            let shape = self.tcx.value_shape_ref(ty);
            if shape == "&NS_SHAPE_LEAF" {
                let _ = writeln!(self.pre, "    {field} = {local};");
            } else {
                let ty_c = self.tcx.c_type(ty);
                let _ = writeln!(
                    self.pre,
                    "    astra_gc_copy_into({task_var}->core.heap, astra_current_heap(), {shape}, sizeof({ty_c}), &{field}, &{local});"
                );
            }
        }
        let _ = writeln!(
            self.pre,
            "    {task_var}->core.value_offset = offsetof({closure_c}, value);"
        );
        let _ = writeln!(self.pre, "    {task_var}->core.entry = &{entry_sym};");
        let _ = writeln!(self.pre, "    {task_var}->core.closure = {closure_var};");
        let _ = writeln!(self.pre, "    astra_task_start(&{task_var}->core);");
        if let Some(scope) = self.scope_tasks.last_mut() {
            scope.push(PendingTask {
                var: task_var.clone(),
                task_ty,
            });
        }
        task_var
    }

    fn emit_spawn_body(
        &mut self,
        body_sym: &str,
        entry_sym: &str,
        closure_c: &str,
        inner: &Type,
        captures: &[(String, Type)],
        body: &ast::Block,
    ) {
        let inner_c = self.tcx.c_type(inner);

        let saved_out = std::mem::take(&mut self.out);
        let saved_pre = std::mem::take(&mut self.pre);
        let saved_roots = self.roots.take();
        let saved_ret = std::mem::replace(&mut self.cur_ret, inner.clone());
        let saved_err = self.cur_err.take();
        let saved_pin = std::mem::replace(&mut self.pin_returns, false);
        let saved_defers = std::mem::take(&mut self.defers);
        let saved_tasks = std::mem::take(&mut self.scope_tasks);
        let saved_expected = self.expected.take();
        let saved_hoist = std::mem::replace(&mut self.hoist, true);

        let mut root_slots = self.count_root_slots(body, &[]);
        for (_, ty) in captures {
            root_slots += self.tcx.root_slot_count(ty);
        }

        let _ = writeln!(self.out, "static {inner_c} {body_sym}({closure_c}* _c) {{");
        self.roots = Some(RootFrame {
            var: "_rf".to_string(),
            used: 0,
            locals: Vec::new(),
            all_locals: Vec::new(),
            capture_scopes: Vec::new(),
        });
        let _ = writeln!(
            self.out,
            "    size_t _rf = astra_gc_frame_enter({root_slots});"
        );
        for (name, ty) in captures {
            let ty_c = self.tcx.c_type(ty);
            let c_name = format!("v_{}", sanitize(name));
            let first = self.roots.as_ref().map_or(0, |frame| frame.used);
            let _ = writeln!(self.out, "    {ty_c} {c_name} = _c->c_{};", sanitize(name));
            let slots = self.publish_local(&c_name, ty, first);
            self.record_local(c_name, ty.clone(), first, slots);
        }
        self.writeln_s("    astra_gc_maybe_collect();");
        self.emit_block_body(body, true, false);
        self.roots = None;
        self.out.push_str("}\n");

        let _ = writeln!(self.out, "static void {entry_sym}(void* _arg) {{");
        let _ = writeln!(self.out, "    {closure_c}* _c = ({closure_c}*)_arg;");
        let _ = writeln!(self.out, "    _c->value = {body_sym}(_c);");
        self.out.push_str("}\n");

        self.task_defs.push_str(&self.out);

        self.out = saved_out;
        self.pre = saved_pre;
        self.roots = saved_roots;
        self.cur_ret = saved_ret;
        self.cur_err = saved_err;
        self.pin_returns = saved_pin;
        self.defers = saved_defers;
        self.scope_tasks = saved_tasks;
        self.expected = saved_expected;
        self.hoist = saved_hoist;
    }

    fn spawn_captures(&self, body: &ast::Block) -> Vec<(String, Type)> {
        let free = astra_core::free_names::spawn_free_names(body);
        let Some(frame) = &self.roots else {
            return Vec::new();
        };
        let mut captures = Vec::new();
        for name in free {
            let c_name = format!("v_{}", sanitize(&name));
            if let Some((_, ty)) = frame
                .all_locals
                .iter()
                .rev()
                .find(|(local, _)| *local == c_name)
            {
                captures.push((name, ty.clone()));
            }
        }
        captures
    }

    fn block_expr_c(&mut self, block: &ast::Block) -> String {
        let ty = self.block_value_type(block);
        let tmp = self.fresh_id("b");
        let ty_c = if ty == Type::Unit {
            "NS_Unit".to_string()
        } else {
            self.tcx.c_type(&ty)
        };
        let zero = self.zero(&ty);
        let _ = writeln!(self.pre, "    {ty_c} {tmp} = {zero};");
        self.pre.push_str("    {\n");
        self.emit_block_value(block, &tmp, &ty);
        self.pre.push_str("    }\n");
        tmp
    }

    fn block_value_type(&self, block: &ast::Block) -> Type {
        block
            .statements
            .last()
            .and_then(|s| match &s.kind {
                StmtKind::Expr(e) => self.type_of(e).cloned(),
                _ => Some(Type::Unit),
            })
            .unwrap_or(Type::Unit)
    }

    fn emit_block_value(&mut self, block: &ast::Block, target: &str, block_ty: &Type) {
        self.defers.push(Vec::new());
        self.scope_tasks.push(Vec::new());
        self.push_capture_scope();
        let n = block.statements.len();
        for (i, stmt) in block.statements.iter().enumerate() {
            let is_tail = i + 1 == n;
            if is_tail {
                if matches!(&stmt.kind, StmtKind::Return { .. }) {
                    self.emit_stmt_into_pre(stmt);
                } else {
                    self.emit_block_tail_into_pre(stmt, target, block_ty);
                }
            } else {
                self.emit_stmt_into_pre(stmt);
            }
        }
        if block.statements.is_empty() && block_ty == &Type::Unit {
            let _ = writeln!(self.pre, "        {target} = (NS_Unit){{0}};");
        }
        self.emit_scope_task_joins_into_pre();
        self.emit_scope_defers_into_pre();
        self.emit_scope_task_joins_into_pre();
        self.scope_tasks.pop();
        self.defers.pop();
        self.pop_capture_scope();
    }

    fn emit_stmt_into_pre(&mut self, stmt: &Stmt) {
        let saved_out = std::mem::take(&mut self.out);
        let saved_pre = std::mem::take(&mut self.pre);
        self.emit_stmt(stmt);
        let mut result = saved_pre;
        result.push_str(&self.out);
        result.push_str(&self.pre);
        self.pre = result;
        self.out = saved_out;
    }

    fn emit_block_tail_into_pre(&mut self, stmt: &Stmt, target: &str, block_ty: &Type) {
        let saved_out = std::mem::take(&mut self.out);
        let saved_pre = std::mem::take(&mut self.pre);
        self.emit_block_tail(stmt, target, block_ty);
        let mut result = saved_pre;
        result.push_str(&self.out);
        result.push_str(&self.pre);
        self.pre = result;
        self.out = saved_out;
    }

    fn emit_block_tail(&mut self, stmt: &Stmt, target: &str, block_ty: &Type) {
        match &stmt.kind {
            StmtKind::Expr(e) => {
                let ty = self.type_of(e).cloned().unwrap_or(Type::Unit);
                if ty == Type::Never {
                    self.emit_expr_stmt(e);
                    return;
                }
                let v = self.expr(e);
                self.flush_pre();
                if block_ty == &Type::Unit {
                    self.emit_value_stmt(e, v, &Some(ty));
                } else {
                    let _ = writeln!(self.out, "    {target} = {v};");
                }
            }
            _ => {
                self.emit_stmt(stmt);
                if block_ty != &Type::Unit {
                    let z = self.zero(block_ty);
                    let _ = writeln!(self.out, "    {target} = {z};");
                }
            }
        }
    }

    fn if_expr_c(
        &mut self,
        condition: &Expr,
        then_branch: &ast::Block,
        else_branch: Option<&ElseBranch>,
        e: &Expr,
    ) -> String {
        let ty = self.type_of(e).cloned().unwrap_or(Type::Unit);
        let tmp = self.fresh_id("if");
        let ty_c = if ty == Type::Unit {
            "NS_Unit".to_string()
        } else {
            self.tcx.c_type(&ty)
        };
        let zero = self.zero(&ty);
        let _ = writeln!(self.pre, "    {ty_c} {tmp} = {zero};");
        let cond = self.expr(condition);
        let _ = writeln!(self.pre, "    if ({cond}) {{");
        self.emit_block_value(then_branch, &tmp, &ty);
        self.pre.push_str("    }");
        if let Some(ElseBranch::Block(b)) = else_branch {
            self.pre.push_str(" else {\n");
            self.emit_block_value(b, &tmp, &ty);
            self.pre.push_str("    }\n");
        } else if let Some(ElseBranch::If(inner)) = else_branch {
            self.pre.push_str(" else { ");
            let inner = self.expr(inner);
            let _ = writeln!(self.pre, "{tmp} = {inner}; }}");
        } else {
            self.pre.push('\n');
        }
        tmp
    }

    fn match_expr_c(&mut self, value: &Expr, arms: &[MatchArm], e: &Expr) -> String {
        let ty = self.type_of(e).cloned().unwrap_or(Type::Unit);
        let tmp = self.fresh_id("m");
        let ty_c = if ty == Type::Unit {
            "NS_Unit".to_string()
        } else {
            self.tcx.c_type(&ty)
        };
        let zero = self.zero(&ty);
        let _ = writeln!(self.pre, "    {ty_c} {tmp} = {zero};");

        let scrut_ty = self
            .materialized_result_type(value)
            .or_else(|| self.type_of(value).cloned())
            .unwrap_or(Type::Unit);
        let scrut = self.expr(value);
        self.flush_pre();

        let scrut = if matches!(
            value.kind,
            ExprKind::Call { .. }
                | ExprKind::Block(_)
                | ExprKind::If { .. }
                | ExprKind::Match { .. }
        ) {
            let scrut_c = self.tcx.c_type(&scrut_ty);
            let scrut_tmp = self.fresh_id("scrut");
            let _ = writeln!(self.pre, "    {scrut_c} {scrut_tmp} = {scrut};");
            self.flush_pre();
            scrut_tmp
        } else {
            scrut
        };
        self.emit_match_dispatch(&scrut, &scrut_ty, arms, Some(&tmp), &ty);
        tmp
    }

    fn materialized_result_type(&self, value: &Expr) -> Option<Type> {
        match self.type_of(value).cloned()? {
            ty @ Type::Result(_, _) => Some(ty),
            base => self
                .effect_error_type(value)
                .map(|error| Type::Result(Box::new(base), Box::new(error))),
        }
    }

    fn effect_error_type(&self, value: &Expr) -> Option<Type> {
        let ExprKind::Call { callee, .. } = &value.kind else {
            return None;
        };
        match &callee.kind {
            ExprKind::Member { target, member } => {
                let receiver = self.type_of(target)?;
                let error = match member.as_str() {
                    "slice" if matches!(receiver, Type::Str) => "SliceError",
                    "to_int" if matches!(receiver, Type::Str) => "ParseIntError",
                    "checked_add" | "checked_sub" | "checked_mul" | "checked_div"
                        if receiver.is_integer() =>
                    {
                        "OverflowError"
                    }
                    "send" | "receive" if matches!(receiver, Type::Named(name, _) if name == "Channel") => {
                        "ChannelError"
                    }
                    _ => return None,
                };
                Some(Type::Named(error.to_owned(), Vec::new()))
            }
            ExprKind::Identifier(name) => {
                let item = self
                    .module
                    .hir
                    .items
                    .iter()
                    .find(|item| item.name == *name)?;
                let astra_core::hir::Declaration::Function(decl) = &item.declaration else {
                    return None;
                };
                Some(path_to_type(decl.error_set.first()?))
            }
            _ => None,
        }
    }

    fn integer_method_c(
        &mut self,
        ty: &Type,
        method: &str,
        receiver: &Expr,
        args: &[Expr],
    ) -> Option<String> {
        let (checked, op) = match method {
            "checked_add" => (true, "add"),
            "checked_sub" => (true, "sub"),
            "checked_mul" => (true, "mul"),
            "checked_div" => (true, "div"),
            "wrapping_add" => (false, "add"),
            "wrapping_sub" => (false, "sub"),
            "wrapping_mul" => (false, "mul"),
            "wrapping_div" => (false, "div"),
            _ => return None,
        };
        let argument = args.first()?;
        let ty_c = self.tcx.c_type(ty);
        let left = self.expr(receiver);
        let right = self.expr(argument);
        let signed = is_signed_int(ty);

        if !checked && op != "div" {
            return Some(match (signed, signed_width(ty)) {
                (true, Some(width)) => format!("astra_{op}_{width}(({left}), ({right}))"),
                _ => {
                    let symbol = match op {
                        "add" => "+",
                        "sub" => "-",
                        _ => "*",
                    };
                    format!("({ty_c})(({left}) {symbol} ({right}))")
                }
            });
        }
        let min = int_min_literal(ty);
        if signed && min.is_none() {
            return None;
        }
        let lhs = self.fresh_id("il");
        let rhs = self.fresh_id("ir");
        let out = self.fresh_id("io");
        let _ = writeln!(self.pre, "    {ty_c} {lhs} = {left};");
        let _ = writeln!(self.pre, "    {ty_c} {rhs} = {right};");
        if checked {
            let ok = self.fresh_id("iok");
            let res = self.fresh_id("ires");
            let res_ty = Type::Result(
                Box::new(ty.clone()),
                Box::new(Type::Named("OverflowError".to_owned(), Vec::new())),
            );
            let res_c = self.tcx.c_type(&res_ty);
            let _ = writeln!(self.pre, "    {ty_c} {out} = 0;");
            if op == "div" {
                let _ = writeln!(
                    self.pre,
                    "    if ({rhs} == 0) astra_panic(\"division by zero\");"
                );
                match min {
                    Some(min) => {
                        let _ = writeln!(
                            self.pre,
                            "    bool {ok} = !({lhs} == {min} && {rhs} == -1);"
                        );
                    }
                    None => {
                        let _ = writeln!(self.pre, "    bool {ok} = true;");
                    }
                }
                let _ = writeln!(self.pre, "    if ({ok}) {{ {out} = {lhs} / {rhs}; }}");
            } else {
                let _ = writeln!(
                    self.pre,
                    "    bool {ok} = !__builtin_{op}_overflow({lhs}, {rhs}, &{out});"
                );
            }
            let _ = writeln!(
                self.pre,
                "    {res_c} {res} = {ok} ? ({res_c}){{ .ok = true, .value = {out} }} : ({res_c}){{ .ok = false, .error = (NS_OverflowError){{ \"integer overflow in '{method}'\" }} }};"
            );
            return Some(res);
        }

        let _ = writeln!(
            self.pre,
            "    if ({rhs} == 0) astra_panic(\"division by zero\");"
        );
        match min {
            Some(min) => {
                let _ = writeln!(
                    self.pre,
                    "    {ty_c} {out} = ({lhs} == {min} && {rhs} == -1) ? {min} : ({lhs} / {rhs});"
                );
            }
            None => {
                let _ = writeln!(self.pre, "    {ty_c} {out} = {lhs} / {rhs};");
            }
        }
        Some(out)
    }

    fn emit_match_dispatch(
        &mut self,
        scrut: &str,
        scrut_ty: &Type,
        arms: &[MatchArm],
        target: Option<&str>,
        result_ty: &Type,
    ) {
        match scrut_ty {
            Type::Option(_) => self.match_option(scrut, scrut_ty, arms, target, result_ty),
            Type::Result(_, _) => self.match_result(scrut, scrut_ty, arms, target, result_ty),
            Type::Named(name, _) if self.tcx.enum_shape(name).is_some() => {
                self.match_enum(scrut, name, arms, target, result_ty)
            }
            Type::Named(name, _) if self.tcx.struct_shape(name).is_some() => {
                self.match_struct(scrut, name, arms, target, result_ty)
            }
            Type::Tuple(_) => self.match_tuple(scrut, scrut_ty, arms, target, result_ty),
            ty if ty.is_numeric() || matches!(ty, Type::Bool | Type::Str | Type::Bytes) => {
                self.match_literal(scrut, scrut_ty, arms, target, result_ty)
            }
            _ => {
                self.tcx.rejects.push(reject(
                    "NS8021",
                    "matching this type is not supported by the C backend first slice",
                ));
            }
        }
    }

    fn match_option(
        &mut self,
        scrut: &str,
        scrut_ty: &Type,
        arms: &[MatchArm],
        target: Option<&str>,
        result_ty: &Type,
    ) {
        let inner = match scrut_ty {
            Type::Option(t) => (**t).clone(),
            _ => Type::Unit,
        };
        let _ = writeln!(self.out, "    if (({scrut}).some) {{");
        for arm in arms {
            if let PatternKind::Constructor { path, arguments } = &arm.pattern.kind
                && path.segments.last().map(String::as_str) == Some("Some")
            {
                if let Some(p) = arguments.first() {
                    self.bind_pattern(p, &format!("({scrut}).value"), &inner);
                }
                self.emit_arm_body(&arm.body, target, result_ty);
            }
        }
        self.writeln_s("    } else {");
        for arm in arms {
            match &arm.pattern.kind {
                PatternKind::Variant { variant, .. } if variant == "None" => {
                    self.emit_arm_body(&arm.body, target, result_ty);
                }
                PatternKind::Identifier(_) | PatternKind::Wildcard => {
                    self.emit_arm_body(&arm.body, target, result_ty);
                }
                _ => {}
            }
        }
        self.writeln_s("    }");
    }

    fn match_result(
        &mut self,
        scrut: &str,
        scrut_ty: &Type,
        arms: &[MatchArm],
        target: Option<&str>,
        result_ty: &Type,
    ) {
        let (ok_t, err_t) = match scrut_ty {
            Type::Result(ok, err) => ((**ok).clone(), (**err).clone()),
            _ => (Type::Unit, Type::Unit),
        };
        let _ = writeln!(self.out, "    if (({scrut}).ok) {{");
        for arm in arms {
            if let PatternKind::Constructor { path, arguments } = &arm.pattern.kind
                && path.segments.last().map(String::as_str) == Some("Ok")
            {
                if let Some(p) = arguments.first() {
                    self.bind_pattern(p, &format!("({scrut}).value"), &ok_t);
                }
                self.emit_arm_body(&arm.body, target, result_ty);
            }
        }
        self.writeln_s("    } else {");
        for arm in arms {
            if let PatternKind::Constructor { path, arguments } = &arm.pattern.kind
                && path.segments.last().map(String::as_str) == Some("Err")
            {
                if let Some(p) = arguments.first() {
                    self.bind_pattern(p, &format!("({scrut}).error"), &err_t);
                }
                self.emit_arm_body(&arm.body, target, result_ty);
            }
        }
        self.writeln_s("    }");
    }

    fn match_enum(
        &mut self,
        scrut: &str,
        name: &str,
        arms: &[MatchArm],
        target: Option<&str>,
        result_ty: &Type,
    ) {
        let _ = writeln!(self.out, "    switch (({scrut}).tag) {{");
        let mut default_arm: Option<usize> = None;
        for (i, arm) in arms.iter().enumerate() {
            match &arm.pattern.kind {
                PatternKind::Variant {
                    path,
                    variant,
                    arguments,
                } => {
                    let owner = if path.segments.is_empty() {
                        name.to_string()
                    } else {
                        path.segments.join(".")
                    };
                    let Some(tag) = self.tcx.enum_tag(&owner, variant) else {
                        self.tcx
                            .rejects
                            .push(reject("NS8020", format!("enum '{owner}' is not declared")));
                        continue;
                    };
                    let _ = writeln!(self.out, "    case {tag}: {{");
                    if let Some(args) = arguments
                        && let Some(shape) = self.tcx.enum_shape(&owner)
                        && let Some(vshape) = shape.variants.iter().find(|v| &v.name == variant)
                    {
                        for (j, subpat) in args.iter().enumerate() {
                            let field_ty = vshape.fields.get(j).cloned().unwrap_or(Type::Unit);
                            let payload = if vshape.fields.len() == 1 {
                                format!("({scrut}).payload.{}", field_c(variant))
                            } else {
                                format!("({scrut}).payload.{}._{j}", field_c(variant))
                            };
                            self.bind_pattern(subpat, &payload, &field_ty);
                        }
                    }
                    self.emit_arm_body(&arm.body, target, result_ty);
                    self.writeln_s("        break;");
                    self.writeln_s("    }");
                }
                PatternKind::Identifier(_) | PatternKind::Wildcard => {
                    default_arm = Some(i);
                }
                _ => {
                    self.tcx.rejects.push(reject(
                        "NS8021",
                        "this enum pattern is not supported by the C backend first slice",
                    ));
                }
            }
        }
        if let Some(i) = default_arm {
            self.writeln_s("    default: {");
            let arm = &arms[i];
            if let PatternKind::Identifier(pat_name) = &arm.pattern.kind {
                let enum_c = self.tcx.c_type(&Type::Named(name.to_string(), Vec::new()));
                let _ = writeln!(
                    self.out,
                    "        {enum_c} v_{} = {scrut};",
                    sanitize(pat_name)
                );
            }
            self.emit_arm_body(&arm.body, target, result_ty);
            self.writeln_s("        break;");
            self.writeln_s("    }");
        }
        self.writeln_s("    }");
    }

    fn match_struct(
        &mut self,
        scrut: &str,
        name: &str,
        arms: &[MatchArm],
        target: Option<&str>,
        result_ty: &Type,
    ) {
        let Some(shape) = self.tcx.struct_shape(name) else {
            self.tcx
                .rejects
                .push(reject("NS8020", format!("struct '{name}' is not declared")));
            return;
        };
        for arm in arms {
            if let PatternKind::Constructor { path, arguments } = &arm.pattern.kind
                && (path.segments.join(".") == name
                    || path.segments.last().map(String::as_str) == Some(name))
            {
                for (j, subpat) in arguments.iter().enumerate() {
                    let (fname, fty) = shape
                        .fields
                        .get(j)
                        .cloned()
                        .unwrap_or((format!("_{j}"), Type::Unit));
                    self.bind_pattern(subpat, &format!("({scrut}).{fname}"), &fty);
                }
                self.emit_arm_body(&arm.body, target, result_ty);
                return;
            }
            if let PatternKind::Identifier(n) = &arm.pattern.kind {
                let s_c = self.tcx.c_type(&Type::Named(name.to_string(), Vec::new()));
                let _ = writeln!(self.out, "    {s_c} v_{} = {scrut};", sanitize(n));
                self.emit_arm_body(&arm.body, target, result_ty);
                return;
            }
            if let PatternKind::Wildcard = &arm.pattern.kind {
                self.emit_arm_body(&arm.body, target, result_ty);
                return;
            }
        }
        self.tcx.rejects.push(reject(
            "NS8021",
            "this struct match is not supported by the C backend first slice",
        ));
    }

    fn match_tuple(
        &mut self,
        scrut: &str,
        scrut_ty: &Type,
        arms: &[MatchArm],
        target: Option<&str>,
        result_ty: &Type,
    ) {
        let Some(PatternKind::Tuple(subpats)) = arms.first().map(|a| &a.pattern.kind) else {
            self.tcx.rejects.push(reject(
                "NS8021",
                "tuple match is not supported by the C backend first slice",
            ));
            return;
        };
        let element_tys = match scrut_ty {
            Type::Tuple(ts) => ts.clone(),
            _ => vec![Type::Unit; subpats.len()],
        };
        for (i, subpat) in subpats.iter().enumerate() {
            let et = element_tys.get(i).cloned().unwrap_or(Type::Unit);
            self.bind_pattern(subpat, &format!("({scrut})._{i}"), &et);
        }
        if let Some(arm) = arms.first() {
            self.emit_arm_body(&arm.body, target, result_ty);
        }
    }

    fn match_literal(
        &mut self,
        scrut: &str,
        scrut_ty: &Type,
        arms: &[MatchArm],
        target: Option<&str>,
        result_ty: &Type,
    ) {
        let mut first = true;
        let mut fallback: Option<usize> = None;
        for (i, arm) in arms.iter().enumerate() {
            match &arm.pattern.kind {
                PatternKind::Literal(lit) => {
                    let lit_c = self.literal_c(lit);
                    let cond = match scrut_ty {
                        Type::Str => format!("astra_str_eq({scrut}, {lit_c})"),
                        Type::Bytes => format!("astra_bytes_eq({scrut}, {lit_c})"),
                        _ => format!("{scrut} == {lit_c}"),
                    };
                    if first {
                        let _ = writeln!(self.out, "    if ({cond}) {{");
                        first = false;
                    } else {
                        let _ = writeln!(self.out, "    }} else if ({cond}) {{");
                    }
                    self.emit_arm_body(&arm.body, target, result_ty);
                }
                PatternKind::Identifier(_) | PatternKind::Wildcard => {
                    fallback = Some(i);
                }
                _ => {
                    self.tcx.rejects.push(reject(
                        "NS8021",
                        "this literal pattern is not supported by the C backend first slice",
                    ));
                }
            }
        }
        if let Some(i) = fallback {
            let arm = &arms[i];
            if first {
                self.bind_pattern(&arm.pattern, scrut, scrut_ty);
                self.emit_arm_body(&arm.body, target, result_ty);
            } else {
                self.writeln_s("    } else {");
                self.bind_pattern(&arm.pattern, scrut, scrut_ty);
                self.emit_arm_body(&arm.body, target, result_ty);
                self.writeln_s("    }");
            }
        } else if !first {
            self.writeln_s("    }");
        }
    }

    fn emit_arm_body(&mut self, body: &MatchArmBody, target: Option<&str>, result_ty: &Type) {
        self.push_capture_scope();
        let pending_locals = std::mem::take(&mut self.pending_pattern_locals);
        let pending_slots = std::mem::take(&mut self.pending_pattern_slots);
        if let Some(frame) = self.roots.as_mut() {
            frame.all_locals.extend(pending_locals);
            frame.locals.extend(pending_slots);
        }
        match body {
            MatchArmBody::Expr(e) => {
                let ty = self.type_of(e).cloned().unwrap_or(Type::Unit);
                if ty == Type::Never {
                    self.emit_expr_stmt(e);
                    self.pop_capture_scope();
                    return;
                }
                let v = self.expr_with_expected(e, result_ty);
                self.flush_pre();
                match target {
                    Some(t) => {
                        if result_ty == &Type::Unit && ty == Type::Unit {
                            self.emit_value_stmt(e, v, &Some(ty));
                        } else {
                            let _ = writeln!(self.out, "        {t} = {v};");
                        }
                    }
                    None => self.emit_value_stmt(e, v, &Some(ty)),
                }
            }
            MatchArmBody::Block(b) => {
                self.writeln_s("        {");
                let n = b.statements.len();
                for (i, stmt) in b.statements.iter().enumerate() {
                    let is_tail = i + 1 == n;
                    if is_tail {
                        self.emit_block_tail_in_arm(stmt, target, result_ty);
                    } else {
                        self.emit_stmt_into_pre(stmt);
                    }

                    self.flush_pre();
                }
                if b.statements.is_empty()
                    && target.is_some()
                    && result_ty == &Type::Unit
                    && let Some(t) = target
                {
                    let _ = writeln!(self.out, "        {t} = (NS_Unit){{0}};");
                }
                self.writeln_s("        }");
            }
        }
        self.pop_capture_scope();
    }

    fn emit_block_tail_in_arm(&mut self, stmt: &Stmt, target: Option<&str>, result_ty: &Type) {
        match &stmt.kind {
            StmtKind::Expr(e) => {
                let ty = self.type_of(e).cloned().unwrap_or(Type::Unit);
                if ty == Type::Never {
                    self.emit_expr_stmt(e);
                    return;
                }
                let v = self.expr_with_expected(e, result_ty);
                self.flush_pre();
                match target {
                    Some(t) => {
                        if result_ty == &Type::Unit && ty == Type::Unit {
                            self.emit_value_stmt(e, v, &Some(ty));
                        } else {
                            let _ = writeln!(self.out, "        {t} = {v};");
                        }
                    }
                    None => self.emit_value_stmt(e, v, &Some(ty)),
                }
            }
            _ => {
                self.emit_stmt_into_pre(stmt);
                self.flush_pre();
                if let Some(t) = target {
                    let z = self.zero(result_ty);
                    let _ = writeln!(self.out, "        {t} = {z};");
                }
            }
        }
    }

    fn bind_pattern(&mut self, pat: &Pattern, c_expr: &str, ty: &Type) {
        match &pat.kind {
            PatternKind::Wildcard => {}
            PatternKind::Identifier(name) => {
                let ty_c = self.tcx.c_type(ty);
                let _ = writeln!(self.out, "        {ty_c} v_{} = {c_expr};", sanitize(name));

                let c_name = format!("v_{}", sanitize(name));
                let first = self.roots.as_ref().map_or(0, |frame| frame.used);
                let slots = self.publish_local(&c_name, ty, first);
                self.record_local(c_name, ty.clone(), first, slots);

                if let Some(frame) = self.roots.as_mut() {
                    if let Some(entry) = frame.all_locals.pop() {
                        self.pending_pattern_locals.push(entry);
                    }
                    if slots > 0
                        && let Some(entry) = frame.locals.pop()
                    {
                        self.pending_pattern_slots.push(entry);
                    }
                }
            }
            PatternKind::Tuple(subpats) => {
                let element_tys = match ty {
                    Type::Tuple(ts) => ts.clone(),
                    _ => vec![Type::Unit; subpats.len()],
                };
                for (i, subpat) in subpats.iter().enumerate() {
                    let et = element_tys.get(i).cloned().unwrap_or(Type::Unit);
                    self.bind_pattern(subpat, &format!("({c_expr})._{i}"), &et);
                }
            }
            PatternKind::Constructor { path, arguments } => {
                let name = path.segments.join(".");
                if let Some(shape) = self.tcx.struct_shape(&name) {
                    for (i, subpat) in arguments.iter().enumerate() {
                        let (_, fty) = shape
                            .fields
                            .get(i)
                            .cloned()
                            .unwrap_or((format!("_{i}"), Type::Unit));
                        self.bind_pattern(subpat, &format!("({c_expr})._{i}"), &fty);
                    }
                } else {
                    self.tcx.rejects.push(reject(
                        "NS8020",
                        format!("struct '{name}' is not declared for this pattern"),
                    ));
                }
            }
            PatternKind::Literal(_) => {
                self.tcx.rejects.push(reject(
                    "NS8021",
                    "literal sub-patterns are not supported in this position by the C backend first slice",
                ));
            }
            PatternKind::Variant { .. } => {
                self.tcx.rejects.push(reject(
                    "NS8021",
                    "nested enum patterns are not supported by the C backend first slice",
                ));
            }
        }
    }

    fn propagate_c(&mut self, inner: &Expr) -> String {
        let result_ty = self.materialized_result_type(inner);
        let inner_c = self.expr(inner);

        let result_c = match &result_ty {
            Some(ty) => self.tcx.c_type(ty),
            None => self.fn_result_c(),
        };
        let tmp = self.freshid("p");
        let _ = writeln!(self.pre, "    {result_c} {tmp} = {inner_c};");
        let fn_result_c = self.fn_result_c();
        let exit = self
            .roots
            .as_ref()
            .map(|frame| format!("astra_gc_frame_exit({}); ", frame.var))
            .unwrap_or_default();

        let mut pins = String::new();
        if self.pin_returns
            && let Some(err_ty) = match &result_ty {
                Some(Type::Result(_, error)) => Some((**error).clone()),
                _ => self.cur_err.clone(),
            }
        {
            for slot in self.tcx.root_exprs(&format!("{tmp}.error"), &err_ty) {
                pins.push_str(&format!("astra_gc_pin({slot}); "));
            }
        }
        let _ = writeln!(
            self.pre,
            "    if (!{tmp}.ok) {{ {pins}{exit}return ({fn_result_c}){{ .ok = false, .error = {tmp}.error }}; }}"
        );
        format!("{tmp}.value")
    }

    fn freshid(&mut self, prefix: &str) -> String {
        self.fresh += 1;
        format!("__{prefix}{}", self.fresh)
    }

    fn fail_c(&mut self, error: &ast::TypePath, arguments: &FailArguments) -> String {
        let name = error.segments.join(".");
        let err_ty = self
            .cur_err
            .clone()
            .unwrap_or_else(|| Type::Named(name.clone(), Vec::new()));
        let err_val = self.construct_error(&name, arguments);
        let fn_result_c = self.fn_result_c();
        if self.pin_returns {
            let err_c = self.tcx.c_type(&err_ty);
            let tmp = self.freshid("err");
            let _ = writeln!(self.out, "    {err_c} {tmp} = {err_val};");
            self.emit_pin_roots(&tmp, &err_ty);
            self.emit_frame_exit();
            let _ = writeln!(
                self.out,
                "    return ({fn_result_c}){{ .ok = false, .error = {tmp} }};"
            );
        } else {
            self.emit_frame_exit();
            let _ = writeln!(
                self.out,
                "    return ({fn_result_c}){{ .ok = false, .error = {err_val} }};"
            );
        }
        "((void*)0)".into()
    }

    fn construct_error(&mut self, name: &str, arguments: &FailArguments) -> String {
        let err_c = self.tcx.c_type(&Type::Named(name.to_string(), Vec::new()));
        match arguments {
            FailArguments::Tuple(args) => {
                let Some(shape) = self.tcx.struct_shape(name) else {
                    self.tcx.rejects.push(reject(
                        "NS8020",
                        format!("error type '{name}' is not declared"),
                    ));
                    return format!("({err_c}){{0}}");
                };
                if shape.tuple {
                    let vals: Vec<String> = args.iter().map(|a| self.expr(a)).collect();
                    let joined = vals.join(", ");
                    format!("({err_c}){{ {joined} }}")
                } else {
                    let mut inits = String::new();
                    for (i, a) in args.iter().enumerate() {
                        let val = self.expr(a);
                        if let Some((fname, _)) = shape.fields.get(i) {
                            let _ = write!(inits, ".{fname} = {val}, ");
                        }
                    }
                    format!("({err_c}){{ {inits}}}")
                }
            }
            FailArguments::Struct(fields) => {
                let mut inits = String::new();
                for f in fields {
                    let val = self.expr(&f.value);
                    let _ = write!(inits, ".{} = {val}, ", field_c(&f.name));
                }
                format!("({err_c}){{ {inits}}}")
            }
        }
    }

    fn panic_c(&mut self, message: &Expr) -> String {
        let msg = self.expr(message);
        let payload = panic_payload(&msg);
        let _ = writeln!(self.out, "    astra_panic({payload});");
        "((void*)0)".into()
    }

    fn abort_c(&mut self, message: &Expr) -> String {
        let msg = self.expr(message);
        let payload = panic_payload(&msg);
        let _ = writeln!(self.out, "    astra_abort({payload});");
        "((void*)0)".into()
    }
}

fn panic_payload(msg_expr: &str) -> String {
    if msg_expr.starts_with("(NS_Str)") {
        format!("({msg_expr}).data")
    } else {
        msg_expr.to_string()
    }
}

fn is_signed_int(ty: &Type) -> bool {
    matches!(
        ty,
        Type::I8 | Type::I16 | Type::I32 | Type::I64 | Type::Isize
    )
}

fn bind_type_params(
    pattern: &Type,
    concrete: &Type,
    parameters: &[String],
    subst: &mut BTreeMap<String, Type>,
) -> Option<()> {
    let bound = match pattern {
        Type::Generic(name) if parameters.iter().any(|parameter| parameter == name) => Some(name),
        Type::Named(name, arguments)
            if arguments.is_empty() && parameters.iter().any(|parameter| parameter == name) =>
        {
            Some(name)
        }
        _ => None,
    };
    if let Some(name) = bound {
        return match subst.get(name) {
            Some(existing) if existing != concrete => None,
            Some(_) => Some(()),
            None => {
                subst.insert(name.clone(), concrete.clone());
                Some(())
            }
        };
    }
    match (pattern, concrete) {
        (Type::Option(a), Type::Option(b))
        | (Type::Array(a), Type::Array(b))
        | (Type::Slice(a), Type::Slice(b)) => bind_type_params(a, b, parameters, subst),
        (Type::FixedArray(a, _), Type::FixedArray(b, _)) => {
            bind_type_params(a, b, parameters, subst)
        }
        (Type::Result(a_ok, a_err), Type::Result(b_ok, b_err)) => {
            bind_type_params(a_ok, b_ok, parameters, subst)?;
            bind_type_params(a_err, b_err, parameters, subst)
        }
        (Type::Map(a_key, a_value), Type::Map(b_key, b_value)) => {
            bind_type_params(a_key, b_key, parameters, subst)?;
            bind_type_params(a_value, b_value, parameters, subst)
        }
        (Type::Tuple(a), Type::Tuple(b)) if a.len() == b.len() => {
            for (x, y) in a.iter().zip(b) {
                bind_type_params(x, y, parameters, subst)?;
            }
            Some(())
        }
        (Type::Named(a_name, a_args), Type::Named(b_name, b_args))
            if a_name == b_name && a_args.len() == b_args.len() =>
        {
            for (x, y) in a_args.iter().zip(b_args) {
                bind_type_params(x, y, parameters, subst)?;
            }
            Some(())
        }
        _ => Some(()),
    }
}

fn signed_width(ty: &Type) -> Option<&'static str> {
    match ty {
        Type::I8 => Some("i8"),
        Type::I16 => Some("i16"),
        Type::I32 => Some("i32"),
        Type::I64 => Some("i64"),
        _ => None,
    }
}

fn int_min_literal(ty: &Type) -> Option<&'static str> {
    match ty {
        Type::I8 => Some("INT8_MIN"),
        Type::I16 => Some("INT16_MIN"),
        Type::I32 => Some("INT32_MIN"),
        Type::I64 => Some("INT64_MIN"),
        _ => None,
    }
}

fn is_shared_capture(ty: &Type) -> bool {
    matches!(
        ty,
        Type::Named(name, _)
            if name == "Channel" || name == "Region" || name == "Ptr"
    )
}
