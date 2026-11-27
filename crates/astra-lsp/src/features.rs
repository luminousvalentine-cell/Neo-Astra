use std::collections::{BTreeMap, BTreeSet};

use astra_core::ast::{self, ItemKind, StmtKind};
use astra_core::hir;
use astra_core::parser::parse;
use astra_core::span::Span;
use astra_core::types::{FunctionType, Type as CoreType};
use serde_json::{Value, json};

use crate::project::{ProjectModule, ProjectSnapshot};
use crate::text::{
    Word, identifier_at, identifier_before_dot, name_span, offset_to_position, position_to_offset,
};

#[derive(Debug, Clone)]
struct Binding {
    name: String,
    span: Span,
    scope: Span,
    kind: &'static str,
    ty: Option<String>,
    value_ty: Option<CoreType>,
}

#[derive(Debug, Clone)]
struct Resolved {
    module_key: String,
    name: String,
    span: Span,
    item: Option<hir::Item>,
    export: Option<hir::Export>,
}

#[must_use]
pub fn hover(snapshot: &ProjectSnapshot, uri: &str, line: u32, character: u32) -> Option<Value> {
    let module = snapshot.module_for_uri(uri)?;
    let offset = position_to_offset(&module.text, line, character);
    let word = identifier_at(&module.text, offset)?;
    let detail = hover_detail(snapshot, module, &word, offset)?;
    let range = span_range_json(&module.text, word.span());
    Some(json!({
        "contents": {
            "kind": "markdown",
            "value": format!("```astra\n{detail}\n```")
        },
        "range": range
    }))
}

#[must_use]
pub fn definition(
    snapshot: &ProjectSnapshot,
    uri: &str,
    line: u32,
    character: u32,
) -> Option<Value> {
    let module = snapshot.module_for_uri(uri)?;
    let offset = position_to_offset(&module.text, line, character);
    let word = identifier_at(&module.text, offset)?;
    let target = reference_target(snapshot, module, &word, offset)?;
    location_json(snapshot, &target)
}

#[must_use]
pub fn completion(snapshot: &ProjectSnapshot, uri: &str, line: u32, character: u32) -> Vec<Value> {
    let Some(module) = snapshot.module_for_uri(uri) else {
        return Vec::new();
    };
    let offset = position_to_offset(&module.text, line, character);
    let start = prefix_start(&module.text, offset);
    let prefix = &module.text[start..offset];

    if let Some((specifier_start, specifier_end)) = import_specifier_range(&module.text, offset) {
        let mut items = BTreeMap::new();
        let prefix = &module.text[specifier_start..specifier_end];
        for (specifier, target) in module_specifiers(snapshot, &module.key) {
            insert_import_completion(
                &mut items,
                &module.text,
                prefix,
                specifier_start,
                specifier_end,
                specifier,
                target,
            );
        }
        if !items.is_empty() {
            return items.into_values().collect();
        }
    }

    let mut items: BTreeMap<String, Value> = BTreeMap::new();
    if let Some(base) = identifier_before_dot(&module.text, start)
        && let Some(detail_items) = member_candidates(snapshot, module, &base)
    {
        for (label, kind, detail) in detail_items {
            insert_completion(&mut items, prefix, label, kind, detail);
        }
        return items.into_values().collect();
    }

    for keyword in KEYWORDS {
        insert_completion(&mut items, prefix, (*keyword).to_owned(), 14, None);
    }
    for builtin in BUILTIN_TYPES {
        insert_completion(&mut items, prefix, (*builtin).to_owned(), 25, None);
    }

    if let Some(ir) = &module.ir {
        for symbol in ir
            .hir
            .symbols
            .values
            .values()
            .chain(ir.hir.symbols.types.values())
            .chain(ir.hir.symbols.namespaces.values())
        {
            insert_completion(
                &mut items,
                prefix,
                symbol.name.clone(),
                completion_kind(symbol.kind),
                Some(format!("from {}", symbol_origin_text(&symbol.origin))),
            );
        }
    }

    for binding in collect_bindings(module) {
        if binding.scope.contains(offset) && binding.name.starts_with(prefix) {
            insert_completion(
                &mut items,
                prefix,
                binding.name,
                6,
                binding.ty.or_else(|| Some(binding.kind.to_owned())),
            );
        }
    }

    items.into_values().collect()
}

#[must_use]
pub fn document_symbols(snapshot: &ProjectSnapshot, uri: &str) -> Vec<Value> {
    let Some(module) = snapshot.module_for_uri(uri) else {
        return Vec::new();
    };
    if let Some(ir) = &module.ir {
        return symbols_for_items(&module.text, &ir.ast.items);
    }
    match parse(&module.text) {
        Ok(ast) => symbols_for_items(&module.text, &ast.items),
        Err(_) => Vec::new(),
    }
}

fn symbols_for_items(text: &str, items: &[ast::Item]) -> Vec<Value> {
    let mut symbols = Vec::new();
    for item in items {
        symbols.extend(symbols_for_item(text, item));
    }
    symbols
}

fn symbols_for_item(text: &str, item: &ast::Item) -> Vec<Value> {
    match &item.kind {
        ItemKind::Function(function) => vec![symbol_json(
            text,
            function.name.clone(),
            12,
            item.span,
            Vec::new(),
        )],
        ItemKind::Struct(declaration) => {
            let children = match &declaration.fields {
                ast::StructFields::Named(fields) => fields
                    .iter()
                    .map(|field| symbol_json(text, field.name.clone(), 8, field.span, Vec::new()))
                    .collect(),
                ast::StructFields::Tuple(_) => Vec::new(),
            };
            vec![symbol_json(
                text,
                declaration.name.clone(),
                23,
                item.span,
                children,
            )]
        }
        ItemKind::Enum(declaration) => {
            let children = declaration
                .variants
                .iter()
                .map(|variant| {
                    symbol_json(text, variant.name.clone(), 22, variant.span, Vec::new())
                })
                .collect();
            vec![symbol_json(
                text,
                declaration.name.clone(),
                10,
                item.span,
                children,
            )]
        }
        ItemKind::Constant(declaration) => vec![symbol_json(
            text,
            declaration.name.clone(),
            14,
            item.span,
            Vec::new(),
        )],
        ItemKind::TypeAlias(declaration) => vec![symbol_json(
            text,
            declaration.name.clone(),
            5,
            item.span,
            Vec::new(),
        )],
        ItemKind::ForeignModule(declaration) => vec![symbol_json(
            text,
            declaration.name.clone(),
            2,
            item.span,
            Vec::new(),
        )],
        ItemKind::ScriptBlock(declaration) => vec![symbol_json(
            text,
            declaration.name.clone(),
            2,
            item.span,
            Vec::new(),
        )],
        ItemKind::ScopedItems(scoped) => symbols_for_items(text, &scoped.items),
        ItemKind::Export(export) => match &export.form {
            ast::ExportForm::DefaultFunction(function) => vec![symbol_json(
                text,
                function.name.clone(),
                12,
                item.span,
                Vec::new(),
            )],
            _ => Vec::new(),
        },
        ItemKind::Import(_) => Vec::new(),
    }
}

fn symbol_json(text: &str, name: String, kind: u32, span: Span, children: Vec<Value>) -> Value {
    let selection = name_span(text, span, &name);
    json!({
        "name": name,
        "kind": kind,
        "range": span_range_json(text, span),
        "selectionRange": span_range_json(text, selection),
        "children": children
    })
}

#[must_use]
pub fn workspace_symbols(snapshot: &ProjectSnapshot, query: &str) -> Vec<Value> {
    let query = query.to_lowercase();
    let mut symbols = Vec::new();
    for module in snapshot.modules.values() {
        for symbol in document_symbols(snapshot, &module.uri) {
            collect_workspace_symbol(
                &module.uri,
                &module.key,
                None,
                &query,
                &symbol,
                &mut symbols,
            );
        }
    }
    symbols.sort_by(|left, right| {
        left["name"]
            .as_str()
            .unwrap_or_default()
            .cmp(right["name"].as_str().unwrap_or_default())
    });
    symbols
}

fn collect_workspace_symbol(
    uri: &str,
    module_key: &str,
    container: Option<&str>,
    query: &str,
    symbol: &Value,
    symbols: &mut Vec<Value>,
) {
    let name = symbol["name"].as_str().unwrap_or_default();
    if query.is_empty() || name.to_lowercase().contains(query) {
        symbols.push(json!({
            "name": name,
            "kind": symbol["kind"],
            "containerName": container.unwrap_or(module_key),
            "location": {
                "uri": uri,
                "range": symbol["range"]
            }
        }));
    }
    if let Some(children) = symbol["children"].as_array() {
        for child in children {
            collect_workspace_symbol(uri, module_key, Some(name), query, child, symbols);
        }
    }
}

#[must_use]
pub fn signature_help(
    snapshot: &ProjectSnapshot,
    uri: &str,
    line: u32,
    character: u32,
) -> Option<Value> {
    let module = snapshot.module_for_uri(uri)?;
    let offset = position_to_offset(&module.text, line, character);
    let ast = if let Some(ir) = &module.ir {
        ir.ast.clone()
    } else {
        parse(&module.text).ok()?
    };
    let call = find_call_at(&ast, offset)?;
    let ast::ExprKind::Call { callee, .. } = &call.kind else {
        return None;
    };
    let callee = unwrap_type_arguments(callee);
    let resolved = resolve_callee(snapshot, module, callee)?;
    let item = resolved.item?;
    let hir::Declaration::Function(function) = item.declaration else {
        return None;
    };
    let parameters: Vec<Value> = function
        .parameters
        .iter()
        .map(|parameter| {
            json!({
                "label": format!("{}: {}", parameter.name, format_ast_type(&parameter.ty))
            })
        })
        .collect();
    let active_parameter = active_parameter(&module.text, callee.span.end, call.span.end, offset);
    Some(json!({
        "signatures": [{
            "label": format_function(&function),
            "parameters": parameters
        }],
        "activeSignature": 0,
        "activeParameter": active_parameter
    }))
}

fn unwrap_type_arguments(expression: &ast::Expr) -> &ast::Expr {
    match &expression.kind {
        ast::ExprKind::TypeArguments { target, .. } => unwrap_type_arguments(target),
        _ => expression,
    }
}

fn resolve_callee(
    snapshot: &ProjectSnapshot,
    module: &ProjectModule,
    callee: &ast::Expr,
) -> Option<Resolved> {
    match &callee.kind {
        ast::ExprKind::Identifier(name) => {
            resolve_name(snapshot, &module.key, name, &mut BTreeSet::new())
        }
        ast::ExprKind::Member { target, member } => {
            let ast::ExprKind::Identifier(base) = &target.kind else {
                return None;
            };
            let base = Word {
                text: base.clone(),
                start: target.span.start,
                end: target.span.end,
            };
            let target_module = namespace_target(module, &base)?;
            resolve_name(snapshot, target_module, member, &mut BTreeSet::new())
        }
        _ => None,
    }
}

fn active_parameter(text: &str, start: usize, end: usize, offset: usize) -> u32 {
    let start = start.min(text.len());
    let end = end.min(text.len());
    let offset = offset.clamp(start, end);
    let mut depth = 0i32;
    let mut active = 0u32;
    let mut in_string = false;
    let mut escaped = false;
    for character in text[start..offset].chars() {
        if in_string {
            if escaped {
                escaped = false;
            } else if character == '\\' {
                escaped = true;
            } else if character == '"' {
                in_string = false;
            }
            continue;
        }
        match character {
            '"' => in_string = true,
            '(' | '[' | '{' => depth += 1,
            ')' | ']' | '}' => depth -= 1,
            ',' if depth == 1 => active += 1,
            _ => {}
        }
    }
    active
}

fn find_call_at(module: &ast::Module, offset: usize) -> Option<&ast::Expr> {
    module
        .items
        .iter()
        .find_map(|item| find_call_in_item(item, offset))
}

fn find_call_in_item(item: &ast::Item, offset: usize) -> Option<&ast::Expr> {
    match &item.kind {
        ItemKind::Function(function) => find_call_in_block(&function.body, offset),
        ItemKind::Export(export) => match &export.form {
            ast::ExportForm::DefaultFunction(function) => {
                find_call_in_block(&function.body, offset)
            }
            ast::ExportForm::DefaultValue(expression) => find_call_in_expr(expression, offset),
            _ => None,
        },
        ItemKind::ScopedItems(scoped) => scoped
            .items
            .iter()
            .find_map(|item| find_call_in_item(item, offset)),
        _ => None,
    }
}

fn find_call_in_block(block: &ast::Block, offset: usize) -> Option<&ast::Expr> {
    block
        .statements
        .iter()
        .find_map(|statement| find_call_in_statement(statement, offset))
}

fn find_call_in_statement(statement: &ast::Stmt, offset: usize) -> Option<&ast::Expr> {
    match &statement.kind {
        StmtKind::Let { value, .. } => find_call_in_expr(value, offset),
        StmtKind::Return { value: Some(value) } | StmtKind::Expr(value) => {
            find_call_in_expr(value, offset)
        }
        StmtKind::Defer(statement) => find_call_in_statement(statement, offset),
        StmtKind::While { condition, body } => {
            find_call_in_expr(condition, offset).or_else(|| find_call_in_block(body, offset))
        }
        StmtKind::For { iterable, body, .. } => {
            find_call_in_expr(iterable, offset).or_else(|| find_call_in_block(body, offset))
        }
        StmtKind::Scoped(scoped) => scoped
            .statements
            .iter()
            .find_map(|statement| find_call_in_statement(statement, offset)),
        StmtKind::Return { value: None } | StmtKind::Break | StmtKind::Continue => None,
    }
}

fn find_call_in_expr(expression: &ast::Expr, offset: usize) -> Option<&ast::Expr> {
    if !span_contains(expression.span, offset) {
        return None;
    }
    let nested = match &expression.kind {
        ast::ExprKind::Call { callee, arguments } => {
            find_call_in_expr(callee, offset).or_else(|| {
                arguments
                    .iter()
                    .find_map(|argument| find_call_in_expr(argument, offset))
            })
        }
        ast::ExprKind::Unary { operand, .. }
        | ast::ExprKind::Propagate(operand)
        | ast::ExprKind::Member {
            target: operand, ..
        }
        | ast::ExprKind::TypeArguments {
            target: operand, ..
        } => find_call_in_expr(operand, offset),
        ast::ExprKind::Binary { left, right, .. }
        | ast::ExprKind::Assign {
            target: left,
            value: right,
        } => find_call_in_expr(left, offset).or_else(|| find_call_in_expr(right, offset)),
        ast::ExprKind::Index { target, index } => {
            find_call_in_expr(target, offset).or_else(|| find_call_in_expr(index, offset))
        }
        ast::ExprKind::Range { start, end, .. } => {
            find_call_in_expr(start, offset).or_else(|| find_call_in_expr(end, offset))
        }
        ast::ExprKind::Array(values) | ast::ExprKind::Tuple(values) => values
            .iter()
            .find_map(|value| find_call_in_expr(value, offset)),
        ast::ExprKind::Map(entries) => entries.iter().find_map(|entry| {
            find_call_in_expr(&entry.key, offset)
                .or_else(|| find_call_in_expr(&entry.value, offset))
        }),
        ast::ExprKind::StructInit { fields, .. } => fields
            .iter()
            .find_map(|field| find_call_in_expr(&field.value, offset)),
        ast::ExprKind::Block(block) => find_call_in_block(block, offset),
        ast::ExprKind::If {
            condition,
            then_branch,
            else_branch,
        } => find_call_in_expr(condition, offset)
            .or_else(|| find_call_in_block(then_branch, offset))
            .or_else(|| {
                else_branch.as_ref().and_then(|branch| match branch {
                    ast::ElseBranch::Block(block) => find_call_in_block(block, offset),
                    ast::ElseBranch::If(expression) => find_call_in_expr(expression, offset),
                })
            }),
        ast::ExprKind::Match { value, arms } => find_call_in_expr(value, offset).or_else(|| {
            arms.iter().find_map(|arm| match &arm.body {
                ast::MatchArmBody::Expr(expression) => find_call_in_expr(expression, offset),
                ast::MatchArmBody::Block(block) => find_call_in_block(block, offset),
            })
        }),
        ast::ExprKind::Spawn { body } => find_call_in_block(body, offset),
        ast::ExprKind::Fail { arguments, .. } => match arguments {
            ast::FailArguments::Tuple(values) => values
                .iter()
                .find_map(|value| find_call_in_expr(value, offset)),
            ast::FailArguments::Struct(fields) => fields
                .iter()
                .find_map(|field| find_call_in_expr(&field.value, offset)),
        },
        ast::ExprKind::Panic { message } | ast::ExprKind::Abort { message } => {
            find_call_in_expr(message, offset)
        }
        ast::ExprKind::Literal(_) | ast::ExprKind::Identifier(_) => None,
    };
    if nested.is_some() {
        return nested;
    }
    match &expression.kind {
        ast::ExprKind::Call { callee, .. } if offset >= callee.span.end => Some(expression),
        _ => None,
    }
}

#[must_use]
pub fn rename(
    snapshot: &ProjectSnapshot,
    uri: &str,
    line: u32,
    character: u32,
    new_name: &str,
) -> Option<Value> {
    if !valid_identifier(new_name) || KEYWORDS.contains(&new_name) {
        return None;
    }
    let module = snapshot.module_for_uri(uri)?;
    let offset = position_to_offset(&module.text, line, character);
    let word = identifier_at(&module.text, offset)?;
    if word.text == new_name {
        return None;
    }
    let target = reference_target(snapshot, module, &word, offset)?;
    resolved_id(snapshot, &target)?;
    if target.item.is_none() && target.export.is_none() && !is_local_binding(snapshot, &target) {
        return None;
    }
    if target_name_exists(snapshot, &target, new_name) {
        return None;
    }

    let references = references(snapshot, uri, line, character, true);
    if references.is_empty() {
        return None;
    }
    let mut changes: BTreeMap<String, Vec<Value>> = BTreeMap::new();
    for reference in references {
        let Some(uri) = reference["uri"].as_str() else {
            continue;
        };
        let Some(range) = reference["range"].as_object() else {
            continue;
        };
        changes.entry(uri.to_owned()).or_default().push(json!({
            "range": range,
            "newText": new_name
        }));
    }
    if changes.is_empty() {
        return None;
    }
    Some(json!({ "changes": changes }))
}

fn is_local_binding(snapshot: &ProjectSnapshot, target: &Resolved) -> bool {
    let Some(module) = snapshot.module(&target.module_key) else {
        return false;
    };
    let Some(target_id) = resolved_id(snapshot, target) else {
        return false;
    };
    collect_bindings(module).iter().any(|binding| {
        let span = name_span(&module.text, binding.span, &binding.name);
        span.start == target_id.start && span.end == target_id.end
    })
}

fn target_name_exists(snapshot: &ProjectSnapshot, target: &Resolved, name: &str) -> bool {
    let Some(module) = snapshot.module(&target.module_key) else {
        return false;
    };
    let Some(ir) = &module.ir else {
        return false;
    };
    ir.hir.items.iter().any(|item| item.name == name)
        || ir.hir.symbols.get(name).is_some()
        || ir.hir.symbols.types.contains_key(name)
        || ir.hir.symbols.namespaces.contains_key(name)
}

fn valid_identifier(value: &str) -> bool {
    let mut characters = value.chars();
    let Some(first) = characters.next() else {
        return false;
    };
    if !(first == '_' || first.is_alphabetic()) {
        return false;
    }
    characters.all(|character| character == '_' || character.is_alphanumeric())
}
#[must_use]
pub fn references(
    snapshot: &ProjectSnapshot,
    uri: &str,
    line: u32,
    character: u32,
    include_declaration: bool,
) -> Vec<Value> {
    let Some(module) = snapshot.module_for_uri(uri) else {
        return Vec::new();
    };
    let offset = position_to_offset(&module.text, line, character);
    let Some(word) = identifier_at(&module.text, offset) else {
        return Vec::new();
    };
    let Some(target) = reference_target(snapshot, module, &word, offset) else {
        return Vec::new();
    };
    let Some(target_id) = resolved_id(snapshot, &target) else {
        return Vec::new();
    };

    let mut locations: BTreeMap<(String, u64, u64, u64, u64), Value> = BTreeMap::new();
    for module in snapshot.modules.values() {
        for word in identifiers(&module.text) {
            if word.text != target.name {
                continue;
            }
            let Some(candidate) = reference_target(snapshot, module, &word, word.start) else {
                continue;
            };
            let Some(candidate_id) = resolved_id(snapshot, &candidate) else {
                continue;
            };
            if candidate_id != target_id {
                continue;
            }
            if !include_declaration && word.start == candidate_id.start {
                continue;
            }
            let location = json!({
                "uri": module.uri,
                "range": span_range_json(&module.text, word.span())
            });
            locations.insert(location_key(&location), location);
        }
    }

    if include_declaration && let Some(location) = location_json(snapshot, &target) {
        locations.insert(location_key(&location), location);
    }

    locations.into_values().collect()
}

#[must_use]
pub fn document_highlight(
    snapshot: &ProjectSnapshot,
    uri: &str,
    line: u32,
    character: u32,
) -> Vec<Value> {
    let Some(module) = snapshot.module_for_uri(uri) else {
        return Vec::new();
    };
    let offset = position_to_offset(&module.text, line, character);
    let Some(word) = identifier_at(&module.text, offset) else {
        return Vec::new();
    };
    let Some(target) = reference_target(snapshot, module, &word, offset) else {
        return Vec::new();
    };
    let Some(target_id) = resolved_id(snapshot, &target) else {
        return Vec::new();
    };
    let mut highlights = BTreeMap::new();
    for word in identifiers(&module.text) {
        if word.text != target.name {
            continue;
        }
        let Some(candidate) = reference_target(snapshot, module, &word, word.start) else {
            continue;
        };
        let Some(candidate_id) = resolved_id(snapshot, &candidate) else {
            continue;
        };
        if candidate_id != target_id {
            continue;
        }
        highlights.insert(
            (word.start, word.end),
            json!({
                "range": span_range_json(&module.text, word.span()),
                "kind": 1
            }),
        );
    }
    highlights.into_values().collect()
}

fn reference_target(
    snapshot: &ProjectSnapshot,
    module: &ProjectModule,
    word: &Word,
    offset: usize,
) -> Option<Resolved> {
    import_specifier_target(snapshot, module, word)
        .or_else(|| field_declaration_target(module, word))
        .or_else(|| definition_target(snapshot, module, word, offset))
}

fn import_specifier_target(
    snapshot: &ProjectSnapshot,
    module: &ProjectModule,
    word: &Word,
) -> Option<Resolved> {
    let ir = module.ir.as_ref()?;
    for import in &ir.hir.imports {
        let bindings = match &import.kind {
            hir::ImportKind::Named { bindings }
            | hir::ImportKind::DefaultNamed { bindings, .. } => bindings,
            _ => continue,
        };
        for binding in bindings {
            let imported_span = name_span(&module.text, binding.span, &binding.imported);
            if imported_span == word.span() {
                return resolve_name(
                    snapshot,
                    &import.module,
                    &binding.imported,
                    &mut BTreeSet::new(),
                );
            }
        }
    }
    None
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
struct SymbolId {
    module_key: String,
    start: usize,
    end: usize,
}

fn resolved_id(snapshot: &ProjectSnapshot, target: &Resolved) -> Option<SymbolId> {
    let module = snapshot.module(&target.module_key)?;
    let span = name_span(&module.text, target.span, &target.name);
    if span.start == span.end {
        return None;
    }
    Some(SymbolId {
        module_key: target.module_key.clone(),
        start: span.start,
        end: span.end,
    })
}

fn field_declaration_target(module: &ProjectModule, word: &Word) -> Option<Resolved> {
    let ir = module.ir.as_ref()?;
    ir.ast
        .items
        .iter()
        .find_map(|item| field_declaration_in_item(module, word, item))
}

fn field_declaration_in_item(
    module: &ProjectModule,
    word: &Word,
    item: &ast::Item,
) -> Option<Resolved> {
    match &item.kind {
        ItemKind::Struct(declaration) => {
            let ast::StructFields::Named(fields) = &declaration.fields else {
                return None;
            };
            for field in fields {
                let span = name_span(&module.text, field.span, &field.name);
                if span == word.span() {
                    return Some(Resolved {
                        module_key: module.key.clone(),
                        name: field.name.clone(),
                        span: field.span,
                        item: None,
                        export: None,
                    });
                }
            }
            None
        }
        ItemKind::ScopedItems(scoped) => scoped
            .items
            .iter()
            .find_map(|item| field_declaration_in_item(module, word, item)),
        _ => None,
    }
}

fn identifiers(text: &str) -> Vec<Word> {
    let mut identifiers = Vec::new();
    let mut index = 0usize;
    let mut in_line_comment = false;
    let mut in_block_comment = false;
    let mut in_string = false;
    let mut escaped = false;
    while index < text.len() {
        let character = text[index..].chars().next().expect("in-bounds char");
        let next = text[index + character.len_utf8()..]
            .chars()
            .next()
            .unwrap_or('\0');
        if in_line_comment {
            if character == '\n' {
                in_line_comment = false;
            }
            index += character.len_utf8();
            continue;
        }
        if in_block_comment {
            if character == '*' && next == '/' {
                in_block_comment = false;
                index += 2;
            } else {
                index += character.len_utf8();
            }
            continue;
        }
        if in_string {
            if escaped {
                escaped = false;
            } else if character == '\\' {
                escaped = true;
            } else if character == '"' {
                in_string = false;
            }
            index += character.len_utf8();
            continue;
        }
        if character == '/' && next == '/' {
            in_line_comment = true;
            index += 2;
            continue;
        }
        if character == '/' && next == '*' {
            in_block_comment = true;
            index += 2;
            continue;
        }
        if character == '"' {
            in_string = true;
            index += character.len_utf8();
            continue;
        }
        if is_identifier_start(character) {
            let start = index;
            index += character.len_utf8();
            while index < text.len() {
                let next = text[index..].chars().next().expect("in-bounds char");
                if !is_identifier_continue(next) {
                    break;
                }
                index += next.len_utf8();
            }
            identifiers.push(Word {
                text: text[start..index].to_owned(),
                start,
                end: index,
            });
            continue;
        }
        index += character.len_utf8();
    }
    identifiers
}

fn is_identifier_start(character: char) -> bool {
    character == '_' || character.is_alphabetic()
}

fn is_identifier_continue(character: char) -> bool {
    character == '_' || character.is_alphanumeric()
}

fn span_contains(span: Span, offset: usize) -> bool {
    span.start <= offset && offset <= span.end
}

fn hover_detail(
    snapshot: &ProjectSnapshot,
    module: &ProjectModule,
    word: &Word,
    offset: usize,
) -> Option<String> {
    if let Some(base) = identifier_before_dot(&module.text, word.start)
        && let Some(detail) = member_detail(snapshot, module, &base, word.text.as_str(), offset)
    {
        return Some(detail);
    }

    if let Some(binding) = visible_binding(module, offset, &word.text) {
        return Some(binding_detail(&binding));
    }

    if let Some(symbol) = module
        .ir
        .as_ref()
        .and_then(|ir| ir.hir.symbols.get(&word.text))
        && let Some(resolved) =
            resolve_name(snapshot, &module.key, &word.text, &mut BTreeSet::new())
    {
        let _ = symbol;
        return Some(resolved_detail(&resolved));
    }

    if let Some(ty) = annotation_at(module, word.span()) {
        return Some(format!("{}: {ty}", word.text));
    }

    if BUILTIN_TYPES.contains(&word.text.as_str()) {
        return Some(format!("builtin type {}", word.text));
    }

    None
}

fn binding_detail(binding: &Binding) -> String {
    let ty = binding.ty.as_deref().unwrap_or("_");
    match binding.kind {
        "parameter" => format!("parameter {}: {ty}", binding.name),
        "for" => format!("for {}: {ty}", binding.name),
        "pattern" => format!("pattern {}: {ty}", binding.name),
        _ => format!("let {}: {ty}", binding.name),
    }
}

fn definition_target(
    snapshot: &ProjectSnapshot,
    module: &ProjectModule,
    word: &Word,
    offset: usize,
) -> Option<Resolved> {
    if let Some(base) = identifier_before_dot(&module.text, word.start)
        && let Some(target) = member_target(snapshot, module, &base, word.text.as_str(), offset)
    {
        return Some(target);
    }

    if let Some(binding) = visible_binding(module, offset, &word.text) {
        return Some(Resolved {
            module_key: module.key.clone(),
            name: binding.name.clone(),
            span: binding.span,
            item: None,
            export: None,
        });
    }

    if let Some(target) = resolve_name(snapshot, &module.key, &word.text, &mut BTreeSet::new()) {
        return Some(target);
    }

    None
}

fn member_detail(
    snapshot: &ProjectSnapshot,
    module: &ProjectModule,
    base: &Word,
    member: &str,
    offset: usize,
) -> Option<String> {
    if let Some(target_module) = namespace_target(module, base)
        && let Some(resolved) = resolve_name(snapshot, target_module, member, &mut BTreeSet::new())
    {
        return Some(resolved_detail(&resolved));
    }

    if let Some((target_module, field)) = struct_field(snapshot, module, base, member, offset) {
        let _ = target_module;
        return Some(format!(
            "field {}: {}",
            field.name,
            format_ast_type(&field.ty)
        ));
    }

    annotation_at(
        module,
        Span::new(offset.saturating_sub(1), offset.saturating_add(1)),
    )
    .map(|ty| format!("{member}: {ty}"))
}

fn member_target(
    snapshot: &ProjectSnapshot,
    module: &ProjectModule,
    base: &Word,
    member: &str,
    offset: usize,
) -> Option<Resolved> {
    if let Some(target_module) = namespace_target(module, base)
        && let Some(resolved) = resolve_name(snapshot, target_module, member, &mut BTreeSet::new())
    {
        return Some(resolved);
    }

    if let Some((target_module, field)) = struct_field(snapshot, module, base, member, offset) {
        return Some(Resolved {
            module_key: target_module,
            name: field.name.clone(),
            span: field.span,
            item: None,
            export: None,
        });
    }
    None
}

fn namespace_target<'a>(module: &'a ProjectModule, base: &Word) -> Option<&'a str> {
    let symbol = module.ir.as_ref()?.hir.symbols.get(&base.text)?;
    match &symbol.origin {
        hir::SymbolOrigin::Imported { module, name } if name == "*" => Some(module),
        _ => None,
    }
}

fn struct_field(
    snapshot: &ProjectSnapshot,
    module: &ProjectModule,
    base: &Word,
    member: &str,
    offset: usize,
) -> Option<(String, ast::FieldDecl)> {
    let ty = base_type(module, base, offset)?;
    let CoreType::Named(name, _) = ty else {
        return None;
    };
    let resolved = resolve_name(snapshot, &module.key, &name, &mut BTreeSet::new())?;
    let item = resolved.item?;
    let hir::Declaration::Struct(declaration) = item.declaration else {
        return None;
    };
    let ast::StructFields::Named(fields) = declaration.fields else {
        return None;
    };
    fields
        .into_iter()
        .find(|field| field.name == member)
        .map(|field| (resolved.module_key, field))
}

fn base_type(module: &ProjectModule, base: &Word, offset: usize) -> Option<CoreType> {
    if let Some(binding) = visible_binding(module, offset, &base.text) {
        if let Some(ty) = &binding.value_ty {
            return Some(ty.clone());
        }
        if let Some(ty) = annotation_at(module, base.span()) {
            return Some(ty);
        }
    }
    if let Some(item) = module
        .ir
        .as_ref()
        .and_then(|ir| ir.hir.items.iter().find(|item| item.name == base.text))
        && let Some(ty) = &item.ty
    {
        return Some(ty.clone());
    }
    annotation_at(module, base.span())
}

fn resolved_detail(resolved: &Resolved) -> String {
    if let Some(item) = &resolved.item {
        return item_detail(item);
    }
    if let Some(export) = &resolved.export {
        return format!("export {}", export.name);
    }
    resolved.name.clone()
}

fn item_detail(item: &hir::Item) -> String {
    match &item.declaration {
        hir::Declaration::Function(function) => format_function(function),
        hir::Declaration::Struct(declaration) => format_struct(declaration),
        hir::Declaration::Enum(declaration) => format_enum(declaration),
        hir::Declaration::Constant(declaration) => {
            format!(
                "const {}: {}",
                declaration.name,
                format_ast_type(&declaration.ty)
            )
        }
        hir::Declaration::TypeAlias(declaration) => {
            let parameters = format_type_parameters(&declaration.type_parameters);
            format!(
                "type {}{parameters} = {}",
                declaration.name,
                format_ast_type(&declaration.ty)
            )
        }
        hir::Declaration::ForeignModule(declaration) => {
            format!("foreign module {}", declaration.name)
        }
        hir::Declaration::ScriptBlock(declaration) => {
            format!("script {}: {}", declaration.name, declaration.language)
        }
    }
}

fn format_function(function: &ast::FunctionDecl) -> String {
    let parameters = function
        .parameters
        .iter()
        .map(|parameter| format!("{}: {}", parameter.name, format_ast_type(&parameter.ty)))
        .collect::<Vec<_>>()
        .join(", ");
    let parameters = format_type_parameters(&function.type_parameters) + "(" + &parameters + ")";
    let return_type = function
        .return_type
        .as_ref()
        .map_or_else(|| "()".to_owned(), format_ast_type);
    let mut signature = format!("fn {}{parameters} -> {return_type}", function.name);
    if !function.error_set.is_empty() {
        signature.push_str(" ! ");
        signature.push_str(
            &function
                .error_set
                .iter()
                .map(format_type_path)
                .collect::<Vec<_>>()
                .join(" | "),
        );
    }
    signature
}

fn format_struct(declaration: &ast::StructDecl) -> String {
    let parameters = format_type_parameters(&declaration.type_parameters);
    let fields = match &declaration.fields {
        ast::StructFields::Named(fields) => fields
            .iter()
            .map(|field| format!("{}: {}", field.name, format_ast_type(&field.ty)))
            .collect::<Vec<_>>()
            .join(", "),
        ast::StructFields::Tuple(fields) => fields
            .iter()
            .map(format_ast_type)
            .collect::<Vec<_>>()
            .join(", "),
    };
    format!("struct {}{parameters} {{ {fields} }}", declaration.name)
}

fn format_enum(declaration: &ast::EnumDecl) -> String {
    let variants = declaration
        .variants
        .iter()
        .map(|variant| {
            let fields = variant
                .fields
                .as_ref()
                .map(|fields| {
                    let values = fields
                        .iter()
                        .map(|field| {
                            let name = field
                                .name
                                .as_ref()
                                .map_or_else(String::new, |name| format!("{name}: "));
                            format!("{name}{}", format_ast_type(&field.ty))
                        })
                        .collect::<Vec<_>>()
                        .join(", ");
                    format!("({values})")
                })
                .unwrap_or_default();
            format!("{}{fields}", variant.name)
        })
        .collect::<Vec<_>>()
        .join(", ");
    format!(
        "enum {}{} {{ {variants} }}",
        declaration.name,
        format_type_parameters(&declaration.type_parameters)
    )
}

fn format_type_parameters(parameters: &[String]) -> String {
    if parameters.is_empty() {
        String::new()
    } else {
        format!("<{}>", parameters.join(", "))
    }
}

fn format_ast_type(ty: &ast::Type) -> String {
    match &ty.kind {
        ast::TypeKind::Path(path) => format_type_path(path),
        ast::TypeKind::Array(element) => format!("[{}]", format_ast_type(element)),
        ast::TypeKind::FixedArray(element, length) => {
            format!("[{}; {length}]", format_ast_type(element))
        }
        ast::TypeKind::Map(key, value) => {
            format!("{{{}: {}}}", format_ast_type(key), format_ast_type(value))
        }
        ast::TypeKind::Tuple(elements) => {
            let values = elements
                .iter()
                .map(format_ast_type)
                .collect::<Vec<_>>()
                .join(", ");
            format!("({values})")
        }
        ast::TypeKind::Function {
            parameters,
            return_type,
            error_set,
        } => {
            let parameters = parameters
                .iter()
                .map(format_ast_type)
                .collect::<Vec<_>>()
                .join(", ");
            let mut rendered = format!("fn({parameters})");
            if let Some(return_type) = return_type {
                rendered.push_str(" -> ");
                rendered.push_str(&format_ast_type(return_type));
            }
            if !error_set.is_empty() {
                rendered.push_str(" ! ");
                rendered.push_str(
                    &error_set
                        .iter()
                        .map(format_type_path)
                        .collect::<Vec<_>>()
                        .join(" | "),
                );
            }
            rendered
        }
        ast::TypeKind::Unit => "()".to_owned(),
    }
}

fn format_type_path(path: &ast::TypePath) -> String {
    let mut rendered = path.segments.join(".");
    if !path.arguments.is_empty() {
        rendered.push('<');
        rendered.push_str(
            &path
                .arguments
                .iter()
                .map(format_ast_type)
                .collect::<Vec<_>>()
                .join(", "),
        );
        rendered.push('>');
    }
    rendered
}

fn resolve_name(
    snapshot: &ProjectSnapshot,
    module_key: &str,
    name: &str,
    visited: &mut BTreeSet<(String, String)>,
) -> Option<Resolved> {
    if !visited.insert((module_key.to_owned(), name.to_owned())) {
        return None;
    }
    let module = snapshot.module(module_key)?;
    let ir = module.ir.as_ref();

    if let Some(ir) = ir {
        if let Some(symbol) = ir.hir.symbols.get(name) {
            match &symbol.origin {
                hir::SymbolOrigin::Local => {
                    let item = ir.hir.items.iter().find(|item| item.name == name).cloned();
                    return Some(Resolved {
                        module_key: module_key.to_owned(),
                        name: name.to_owned(),
                        span: symbol.span,
                        item,
                        export: None,
                    });
                }
                hir::SymbolOrigin::Imported {
                    module,
                    name: imported,
                } => {
                    if imported == "*" {
                        return module_resolved(snapshot, module);
                    }
                    return resolve_name(snapshot, module, imported, visited);
                }
            }
        }

        if name == "default"
            && let Some(export) = &ir.hir.exports.default
        {
            return resolve_export(snapshot, module_key, export, visited);
        }
        if let Some(export) = ir.hir.exports.get_named(name) {
            return resolve_export(snapshot, module_key, export, visited);
        }
    }

    if let Some(item) = ir
        .and_then(|ir| ir.hir.items.iter().find(|item| item.name == name))
        .cloned()
    {
        return Some(Resolved {
            module_key: module_key.to_owned(),
            name: name.to_owned(),
            span: item.span,
            item: Some(item),
            export: None,
        });
    }

    if let Some(ir) = ir {
        for star in &ir.hir.exports.stars {
            if let Some(resolved) = resolve_name(snapshot, &star.module, name, visited) {
                return Some(resolved);
            }
        }
    }
    None
}

fn resolve_export(
    snapshot: &ProjectSnapshot,
    module_key: &str,
    export: &hir::Export,
    visited: &mut BTreeSet<(String, String)>,
) -> Option<Resolved> {
    match &export.target {
        hir::ExportTarget::Local(local) => {
            resolve_name(snapshot, module_key, local, visited).map(|mut resolved| {
                resolved.export = Some(export.clone());
                resolved
            })
        }
        hir::ExportTarget::Reexport { module, name } => {
            resolve_name(snapshot, module, name, visited)
        }
        hir::ExportTarget::Namespace { module } => module_resolved(snapshot, module),
        hir::ExportTarget::Expression => Some(Resolved {
            module_key: module_key.to_owned(),
            name: export.name.clone(),
            span: export.span,
            item: None,
            export: Some(export.clone()),
        }),
    }
}

fn module_resolved(snapshot: &ProjectSnapshot, module_key: &str) -> Option<Resolved> {
    let module = snapshot.module(module_key)?;
    Some(Resolved {
        module_key: module_key.to_owned(),
        name: module
            .path
            .file_stem()
            .and_then(|stem| stem.to_str())
            .unwrap_or(module_key)
            .to_owned(),
        span: Span::empty(0),
        item: None,
        export: None,
    })
}

fn symbol_origin_text(origin: &hir::SymbolOrigin) -> String {
    match origin {
        hir::SymbolOrigin::Local => "this module".to_owned(),
        hir::SymbolOrigin::Imported { module, name } => format!("{module}#{name}"),
    }
}

fn location_json(snapshot: &ProjectSnapshot, target: &Resolved) -> Option<Value> {
    let module = snapshot.module(&target.module_key)?;
    let span = name_span(&module.text, target.span, &target.name);
    Some(json!({
        "uri": module.uri,
        "range": span_range_json(&module.text, span)
    }))
}

fn location_key(location: &Value) -> (String, u64, u64, u64, u64) {
    (
        location["uri"].as_str().unwrap_or_default().to_owned(),
        location["range"]["start"]["line"]
            .as_u64()
            .unwrap_or_default(),
        location["range"]["start"]["character"]
            .as_u64()
            .unwrap_or_default(),
        location["range"]["end"]["line"]
            .as_u64()
            .unwrap_or_default(),
        location["range"]["end"]["character"]
            .as_u64()
            .unwrap_or_default(),
    )
}

fn span_range_json(text: &str, span: Span) -> Value {
    let (start_line, start_character) = offset_to_position(text, span.start);
    let (end_line, end_character) = offset_to_position(text, span.end);
    json!({
        "start": { "line": start_line, "character": start_character },
        "end": { "line": end_line, "character": end_character }
    })
}

fn insert_completion(
    items: &mut BTreeMap<String, Value>,
    prefix: &str,
    label: String,
    kind: u32,
    detail: Option<String>,
) {
    if !label.starts_with(prefix) || items.contains_key(&label) {
        return;
    }
    let mut item = json!({
        "label": label,
        "kind": kind,
        "insertText": label
    });
    if let Some(detail) = detail
        && let Some(object) = item.as_object_mut()
    {
        object.insert("detail".to_owned(), Value::String(detail));
    }
    items.insert(label, item);
}

fn insert_import_completion(
    items: &mut BTreeMap<String, Value>,
    text: &str,
    prefix: &str,
    start: usize,
    end: usize,
    specifier: String,
    target: String,
) {
    if !specifier.starts_with(prefix) || items.contains_key(&specifier) {
        return;
    }
    let range = span_range_json(text, Span::new(start, end));
    let item = json!({
        "label": specifier,
        "kind": 17,
        "insertText": specifier,
        "filterText": specifier,
        "detail": target,
        "textEdit": {
            "range": range,
            "newText": specifier
        }
    });
    items.insert(specifier, item);
}

fn import_specifier_range(text: &str, offset: usize) -> Option<(usize, usize)> {
    let offset = offset.min(text.len());
    let line_start = text[..offset].rfind('\n').map_or(0, |index| index + 1);
    let line_prefix = &text[line_start..offset];
    let trimmed = line_prefix.trim_start();
    if !(trimmed.starts_with("import") || trimmed.starts_with("export")) {
        return None;
    }
    let quote = line_prefix.rfind('"')?;
    if line_prefix.matches('"').count().is_multiple_of(2) {
        return None;
    }
    let after_quote = &line_prefix[quote + 1..];
    if after_quote.contains('"') {
        return None;
    }
    Some((line_start + quote + 1, offset))
}

fn module_specifiers(snapshot: &ProjectSnapshot, current_key: &str) -> Vec<(String, String)> {
    let mut specifiers = Vec::new();
    for key in snapshot.modules.keys() {
        if key == current_key {
            continue;
        }
        if let Some(specifier) = relative_module_specifier(current_key, key) {
            specifiers.push((specifier, key.clone()));
        }
    }
    specifiers.sort();
    specifiers
}

fn relative_module_specifier(current_key: &str, target_key: &str) -> Option<String> {
    let mut current_parts: Vec<&str> = current_key.split('/').collect();
    current_parts.pop();
    let target_parts: Vec<&str> = target_key.split('/').collect();
    let mut common = 0;
    while common < current_parts.len()
        && common < target_parts.len()
        && current_parts[common] == target_parts[common]
    {
        common += 1;
    }
    let mut parts = vec![".."; current_parts.len() - common];
    parts.extend(&target_parts[common..]);
    if parts.is_empty() {
        return None;
    }
    let joined = parts.join("/");
    if joined.starts_with("..") {
        Some(joined)
    } else {
        Some(format!("./{joined}"))
    }
}

fn completion_kind(kind: hir::SymbolKind) -> u32 {
    match kind {
        hir::SymbolKind::Function => 3,
        hir::SymbolKind::Struct | hir::SymbolKind::Enum => 22,
        hir::SymbolKind::Constant => 21,
        hir::SymbolKind::TypeAlias | hir::SymbolKind::ImportedType => 25,
        hir::SymbolKind::ForeignModule
        | hir::SymbolKind::ScriptBlock
        | hir::SymbolKind::ImportedNamespace => 9,
        hir::SymbolKind::ImportedValue => 6,
    }
}

fn prefix_start(text: &str, offset: usize) -> usize {
    let offset = offset.min(text.len());
    let mut start = offset;
    while start > 0 {
        let previous = text[..start].chars().next_back().expect("in-bounds char");
        if !previous.is_alphanumeric() && previous != '_' {
            break;
        }
        start -= previous.len_utf8();
    }
    start
}

fn visible_binding(module: &ProjectModule, offset: usize, name: &str) -> Option<Binding> {
    collect_bindings(module)
        .into_iter()
        .filter(|binding| binding.name == name)
        .filter(|binding| {
            binding.span.contains(offset)
                || (binding.scope.contains(offset) && binding.span.start <= offset)
        })
        .max_by_key(|binding| (binding.scope.start, binding.span.start))
}

fn annotation_at(module: &ProjectModule, span: Span) -> Option<CoreType> {
    let ir = module.ir.as_ref()?;
    if let Some(annotation) = ir
        .annotations
        .iter()
        .find(|annotation| annotation.span == span)
    {
        return Some(annotation.ty.clone());
    }
    ir.annotations
        .iter()
        .filter(|annotation| {
            annotation.span.contains(span.start)
                || annotation.span.contains(span.end.saturating_sub(1))
        })
        .min_by_key(|annotation| annotation.span.len())
        .map(|annotation| annotation.ty.clone())
}

fn collect_bindings(module: &ProjectModule) -> Vec<Binding> {
    let Some(ir) = &module.ir else {
        return Vec::new();
    };
    let mut bindings = Vec::new();
    for item in &ir.ast.items {
        collect_item_bindings(module, item, &mut bindings);
    }
    bindings
}

fn collect_item_bindings(module: &ProjectModule, item: &ast::Item, bindings: &mut Vec<Binding>) {
    match &item.kind {
        ItemKind::Function(function) => {
            for parameter in &function.parameters {
                push_binding(
                    module,
                    bindings,
                    &parameter.name,
                    parameter.span,
                    function.body.span,
                    "parameter",
                    Some(&parameter.ty),
                    None,
                );
            }
            collect_block_bindings(module, &function.body, bindings);
        }
        ItemKind::Export(export) => {
            if let ast::ExportForm::DefaultFunction(function) = &export.form {
                for parameter in &function.parameters {
                    push_binding(
                        module,
                        bindings,
                        &parameter.name,
                        parameter.span,
                        function.body.span,
                        "parameter",
                        Some(&parameter.ty),
                        None,
                    );
                }
                collect_block_bindings(module, &function.body, bindings);
            }
        }
        ItemKind::ScopedItems(scoped) => {
            for item in &scoped.items {
                collect_item_bindings(module, item, bindings);
            }
        }
        _ => {}
    }
}

fn collect_block_bindings(module: &ProjectModule, block: &ast::Block, bindings: &mut Vec<Binding>) {
    for statement in &block.statements {
        collect_statement_bindings(module, statement, block.span, bindings);
    }
}

fn collect_statement_bindings(
    module: &ProjectModule,
    statement: &ast::Stmt,
    scope: Span,
    bindings: &mut Vec<Binding>,
) {
    match &statement.kind {
        StmtKind::Let {
            name, ty, value, ..
        } => {
            push_binding(
                module,
                bindings,
                name,
                statement.span,
                scope,
                "let",
                ty.as_ref(),
                annotation_at(module, value.span),
            );
            collect_expr_bindings(module, value, bindings);
        }
        StmtKind::For {
            binding,
            iterable,
            body,
        } => {
            let element = annotation_at(module, iterable.span).and_then(|ty| match ty {
                CoreType::Array(inner) | CoreType::Slice(inner) => Some(*inner),
                CoreType::FixedArray(inner, _) => Some(*inner),
                _ => None,
            });
            push_binding(
                module,
                bindings,
                binding,
                statement.span,
                body.span,
                "for",
                None,
                element,
            );
            collect_expr_bindings(module, iterable, bindings);
            collect_block_bindings(module, body, bindings);
        }
        StmtKind::While { condition, body } => {
            collect_expr_bindings(module, condition, bindings);
            collect_block_bindings(module, body, bindings);
        }
        StmtKind::Return { value: Some(value) } | StmtKind::Expr(value) => {
            collect_expr_bindings(module, value, bindings);
        }
        StmtKind::Defer(statement) => {
            collect_statement_bindings(module, statement, scope, bindings);
        }
        StmtKind::Scoped(scoped) => {
            for statement in &scoped.statements {
                collect_statement_bindings(module, statement, scoped.span, bindings);
            }
        }
        StmtKind::Return { value: None } | StmtKind::Break | StmtKind::Continue => {}
    }
}

fn collect_expr_bindings(module: &ProjectModule, expr: &ast::Expr, bindings: &mut Vec<Binding>) {
    match &expr.kind {
        ast::ExprKind::Block(block) => collect_block_bindings(module, block, bindings),
        ast::ExprKind::If {
            condition,
            then_branch,
            else_branch,
        } => {
            collect_expr_bindings(module, condition, bindings);
            collect_block_bindings(module, then_branch, bindings);
            if let Some(branch) = else_branch {
                match branch {
                    ast::ElseBranch::Block(block) => {
                        collect_block_bindings(module, block, bindings)
                    }
                    ast::ElseBranch::If(expr) => collect_expr_bindings(module, expr, bindings),
                }
            }
        }
        ast::ExprKind::Match { value, arms } => {
            collect_expr_bindings(module, value, bindings);
            for arm in arms {
                let body_span = match &arm.body {
                    ast::MatchArmBody::Expr(expr) => expr.span,
                    ast::MatchArmBody::Block(block) => block.span,
                };
                collect_pattern_bindings(module, &arm.pattern, body_span, bindings);
                match &arm.body {
                    ast::MatchArmBody::Expr(expr) => collect_expr_bindings(module, expr, bindings),
                    ast::MatchArmBody::Block(block) => {
                        collect_block_bindings(module, block, bindings);
                    }
                }
            }
        }
        ast::ExprKind::Spawn { body } => collect_block_bindings(module, body, bindings),
        ast::ExprKind::Unary { operand, .. }
        | ast::ExprKind::Propagate(operand)
        | ast::ExprKind::Member {
            target: operand, ..
        }
        | ast::ExprKind::TypeArguments {
            target: operand, ..
        } => {
            collect_expr_bindings(module, operand, bindings);
        }
        ast::ExprKind::Binary { left, right, .. }
        | ast::ExprKind::Assign {
            target: left,
            value: right,
        } => {
            collect_expr_bindings(module, left, bindings);
            collect_expr_bindings(module, right, bindings);
        }
        ast::ExprKind::Call { callee, arguments } => {
            collect_expr_bindings(module, callee, bindings);
            for argument in arguments {
                collect_expr_bindings(module, argument, bindings);
            }
        }
        ast::ExprKind::Index { target, index } => {
            collect_expr_bindings(module, target, bindings);
            collect_expr_bindings(module, index, bindings);
        }
        ast::ExprKind::Range { start, end, .. } => {
            collect_expr_bindings(module, start, bindings);
            collect_expr_bindings(module, end, bindings);
        }
        ast::ExprKind::Array(values) | ast::ExprKind::Tuple(values) => {
            for value in values {
                collect_expr_bindings(module, value, bindings);
            }
        }
        ast::ExprKind::Map(entries) => {
            for entry in entries {
                collect_expr_bindings(module, &entry.key, bindings);
                collect_expr_bindings(module, &entry.value, bindings);
            }
        }
        ast::ExprKind::StructInit { fields, .. } => {
            for field in fields {
                collect_expr_bindings(module, &field.value, bindings);
            }
        }
        ast::ExprKind::Fail { arguments, .. } => match arguments {
            ast::FailArguments::Tuple(values) => {
                for value in values {
                    collect_expr_bindings(module, value, bindings);
                }
            }
            ast::FailArguments::Struct(fields) => {
                for field in fields {
                    collect_expr_bindings(module, &field.value, bindings);
                }
            }
        },
        ast::ExprKind::Panic { message } | ast::ExprKind::Abort { message } => {
            collect_expr_bindings(module, message, bindings);
        }
        ast::ExprKind::Literal(_) | ast::ExprKind::Identifier(_) => {}
    }
}

fn collect_pattern_bindings(
    module: &ProjectModule,
    pattern: &ast::Pattern,
    scope: Span,
    bindings: &mut Vec<Binding>,
) {
    match &pattern.kind {
        ast::PatternKind::Identifier(name) => {
            push_binding(
                module,
                bindings,
                name,
                pattern.span,
                scope,
                "pattern",
                None,
                None,
            );
        }
        ast::PatternKind::Tuple(patterns)
        | ast::PatternKind::Constructor {
            arguments: patterns,
            ..
        } => {
            for pattern in patterns {
                collect_pattern_bindings(module, pattern, scope, bindings);
            }
        }
        ast::PatternKind::Variant { arguments, .. } => {
            if let Some(patterns) = arguments {
                for pattern in patterns {
                    collect_pattern_bindings(module, pattern, scope, bindings);
                }
            }
        }
        ast::PatternKind::Wildcard | ast::PatternKind::Literal(_) => {}
    }
}

#[allow(clippy::too_many_arguments)]
fn push_binding(
    module: &ProjectModule,
    bindings: &mut Vec<Binding>,
    name: &str,
    declaration_span: Span,
    scope: Span,
    kind: &'static str,
    explicit_type: Option<&ast::Type>,
    inferred_type: Option<CoreType>,
) {
    let span = name_span(&module.text, declaration_span, name);
    let value_ty = explicit_type
        .and_then(ast_type_to_core)
        .or_else(|| inferred_type.clone());
    let ty = explicit_type
        .map(format_ast_type)
        .or_else(|| inferred_type.map(|ty| ty.to_string()));
    bindings.push(Binding {
        name: name.to_owned(),
        span,
        scope,
        kind,
        ty,
        value_ty,
    });
}

fn ast_type_to_core(ty: &ast::Type) -> Option<CoreType> {
    match &ty.kind {
        ast::TypeKind::Path(path) if path.arguments.is_empty() => match path.segments.as_slice() {
            [name] => Some(match name.as_str() {
                "i8" => CoreType::I8,
                "i16" => CoreType::I16,
                "i32" => CoreType::I32,
                "i64" => CoreType::I64,
                "u8" => CoreType::U8,
                "u16" => CoreType::U16,
                "u32" => CoreType::U32,
                "u64" => CoreType::U64,
                "usize" => CoreType::Usize,
                "isize" => CoreType::Isize,
                "f32" => CoreType::F32,
                "f64" => CoreType::F64,
                "bool" => CoreType::Bool,
                "str" => CoreType::Str,
                "bytes" => CoreType::Bytes,
                "Never" => CoreType::Never,
                other => CoreType::Named(other.to_owned(), Vec::new()),
            }),
            _ => Some(CoreType::Named(path.segments.join("."), Vec::new())),
        },
        ast::TypeKind::Array(element) => {
            Some(CoreType::Array(Box::new(ast_type_to_core(element)?)))
        }
        ast::TypeKind::FixedArray(element, length) => Some(CoreType::FixedArray(
            Box::new(ast_type_to_core(element)?),
            length.parse().ok()?,
        )),
        ast::TypeKind::Map(key, value) => Some(CoreType::Map(
            Box::new(ast_type_to_core(key)?),
            Box::new(ast_type_to_core(value)?),
        )),
        ast::TypeKind::Tuple(elements) => Some(CoreType::Tuple(
            elements.iter().filter_map(ast_type_to_core).collect(),
        )),
        ast::TypeKind::Function {
            parameters,
            return_type,
            error_set,
        } => Some(CoreType::Function(FunctionType {
            type_parameters: Vec::new(),
            parameters: parameters.iter().filter_map(ast_type_to_core).collect(),
            return_type: Box::new(
                return_type
                    .as_deref()
                    .and_then(ast_type_to_core)
                    .unwrap_or(CoreType::Unit),
            ),
            errors: error_set
                .iter()
                .map(|path| CoreType::Named(path.segments.join("."), Vec::new()))
                .collect(),
        })),
        ast::TypeKind::Unit => Some(CoreType::Unit),
        ast::TypeKind::Path(_) => None,
    }
}

fn member_candidates(
    snapshot: &ProjectSnapshot,
    module: &ProjectModule,
    base: &Word,
) -> Option<Vec<(String, u32, Option<String>)>> {
    let mut items = Vec::new();
    if let Some(target_module) = namespace_target(module, base) {
        let target = snapshot.module(target_module)?;
        if let Some(ir) = &target.ir {
            for symbol in ir
                .hir
                .symbols
                .values
                .values()
                .chain(ir.hir.symbols.types.values())
                .chain(ir.hir.symbols.namespaces.values())
            {
                items.push((
                    symbol.name.clone(),
                    completion_kind(symbol.kind),
                    Some(symbol_origin_text(&symbol.origin)),
                ));
            }
        }
        return Some(items);
    }

    if let Some(ty) = base_type(module, base, base.start) {
        let CoreType::Named(name, _) = ty else {
            return Some(items);
        };
        let resolved = resolve_name(snapshot, &module.key, &name, &mut BTreeSet::new())?;
        let Some(item) = resolved.item else {
            return Some(items);
        };
        let hir::Declaration::Struct(declaration) = item.declaration else {
            return Some(items);
        };
        if let ast::StructFields::Named(fields) = declaration.fields {
            for field in fields {
                items.push((field.name, 5, Some(format_ast_type(&field.ty))));
            }
        }
        return Some(items);
    }
    None
}

const KEYWORDS: &[&str] = &[
    "break", "const", "continue", "defer", "else", "enum", "export", "false", "fn", "for", "if",
    "import", "let", "match", "module", "mut", "panic", "pub", "return", "spawn", "struct", "true",
    "type", "while",
];

const BUILTIN_TYPES: &[&str] = &[
    "Never", "Option", "Result", "bool", "bytes", "f32", "f64", "i8", "i16", "i32", "i64", "isize",
    "str", "u8", "u16", "u32", "u64", "usize",
];
