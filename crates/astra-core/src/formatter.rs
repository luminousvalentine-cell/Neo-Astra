use crate::ast::*;
use crate::lexer::{Comment, lex};
use crate::parser::{ParseResult, parse_with_comments};

pub fn format_source(source: &str) -> ParseResult<String> {
    let (module, comments) = parse_with_comments(source)?;
    Ok(format_module_with_comments(&module, source, &comments))
}

#[must_use]
pub fn format_module(module: &Module, source: &str) -> String {
    let comments = lex(source).comments;
    format_module_with_comments(module, source, &comments)
}

#[must_use]
pub fn format_module_with_comments(module: &Module, source: &str, comments: &[Comment]) -> String {
    Formatter::new(source, comments).format_module(module)
}

struct Formatter<'a> {
    line_starts: Vec<usize>,
    output: String,
    current_indent: usize,
    comments: &'a [Comment],
    comment_index: usize,
    consumed_comments: Vec<bool>,
    source_len: usize,
}

impl<'a> Formatter<'a> {
    fn new(source: &'a str, comments: &'a [Comment]) -> Self {
        let mut line_starts = vec![0];
        for (index, byte) in source.bytes().enumerate() {
            if byte == b'\n' {
                line_starts.push(index + 1);
            }
        }

        Self {
            line_starts,
            output: String::new(),
            current_indent: 0,
            comments,
            comment_index: 0,
            consumed_comments: vec![false; comments.len()],
            source_len: source.len(),
        }
    }

    fn format_module(mut self, module: &Module) -> String {
        let first_code_line = if let Some(attribute) = module.attributes.first() {
            self.line_of(attribute.span.start)
        } else if !module.name.is_empty() {
            self.line_of(module.header_span.end)
        } else if let Some(item) = module.items.first() {
            self.line_of(item.span.start)
        } else {
            usize::MAX
        };
        let prefix_end = self
            .line_starts
            .get(first_code_line)
            .copied()
            .unwrap_or(self.source_len);
        self.write_comments_before(prefix_end, 0);

        for (index, attribute) in module.attributes.iter().enumerate() {
            self.write_comments_before(attribute.span.start, 0);
            self.ensure_indent(0);
            self.write_attribute(attribute, 0);
            let next_start = module
                .attributes
                .get(index + 1)
                .map_or(module.header_span.start, |next| next.span.start);
            self.write_trailing_comment(attribute.span.end, next_start);
            self.output.push('\n');
        }
        if !module.attributes.is_empty() {
            self.write_comments_before(module.header_span.start, 0);
        }

        if !module.name.is_empty() {
            self.output.push_str("module ");
            self.output.push_str(&module.name.join("."));
            let limit = module
                .items
                .first()
                .map_or(self.source_len, |item| item.span.start);
            self.write_trailing_comment(module.header_span.end, limit);
            self.output.push('\n');
        }

        if !module.items.is_empty() {
            if !module.name.is_empty() {
                self.output.push('\n');
            }
            self.write_items(&module.items, 0, true, self.source_len);
        }

        if !self.output.ends_with('\n') {
            self.output.push('\n');
        }

        self.write_comments_before(self.source_len, 0);
        if !self.output.ends_with('\n') {
            self.output.push('\n');
        }

        self.output
    }

    fn write_items(&mut self, items: &[Item], indent: usize, top_level: bool, end: usize) {
        for (index, item) in items.iter().enumerate() {
            self.write_comments_before(item.span.start, indent);
            self.write_item(item, indent);

            let next_start = items.get(index + 1).map_or(end, |next| next.span.start);
            let trailing = self.write_trailing_comment(item.span.end, next_start);

            if index + 1 == items.len() {
                self.output.push('\n');
            } else if trailing {
                self.output.push('\n');
                if top_level {
                    self.output.push('\n');
                }
            } else if self.same_line(item.span.end, items[index + 1].span.start) {
                self.output.push_str("; ");
            } else {
                self.output.push('\n');
                if top_level {
                    self.output.push('\n');
                }
            }
        }

        self.write_comments_before(end, indent);
    }

    fn write_item(&mut self, item: &Item, indent: usize) {
        for attribute in &item.attributes {
            self.ensure_indent(indent);
            self.write_attribute(attribute, indent);
            self.output.push('\n');
        }

        self.ensure_indent(indent);

        match &item.kind {
            ItemKind::Import(import) => self.write_import(import),
            ItemKind::Export(export) => self.write_export(export, indent),
            ItemKind::ScopedItems(scoped) => self.write_scoped_items(scoped, indent),
            ItemKind::Function(function) => {
                self.write_visibility(item);
                self.write_function(function, indent);
            }
            ItemKind::Struct(declaration) => {
                self.write_visibility(item);
                self.write_struct(declaration, indent);
            }
            ItemKind::Enum(declaration) => {
                self.write_visibility(item);
                self.write_enum(declaration, indent);
            }
            ItemKind::Constant(declaration) => {
                self.write_visibility(item);
                self.write_constant(declaration);
            }
            ItemKind::TypeAlias(declaration) => {
                self.write_visibility(item);
                self.write_type_alias(declaration);
            }
            ItemKind::ForeignModule(declaration) => {
                self.write_visibility(item);
                self.write_foreign_module(declaration, indent);
            }
            ItemKind::ScriptBlock(script) => {
                self.write_script_block(script, indent);
            }
        }
    }

    fn write_visibility(&mut self, item: &Item) {
        if item.exported {
            self.output.push_str("export ");
        } else {
            match item.visibility {
                Visibility::Normal => {}
                Visibility::Public => self.output.push_str("pub "),
                Visibility::Private => self.output.push_str("private "),
            }
        }
    }

    fn write_function(&mut self, function: &FunctionDecl, indent: usize) {
        self.output.push_str("fn ");
        self.output.push_str(&function.name);
        self.write_type_parameters(&function.type_parameters);
        self.output.push('(');
        for (index, parameter) in function.parameters.iter().enumerate() {
            if index > 0 {
                self.output.push_str(", ");
            }
            self.output.push_str(&parameter.name);
            self.output.push_str(": ");
            self.write_type(&parameter.ty);
        }
        self.output.push(')');
        self.write_return_and_errors(&function.return_type, &function.error_set);
        self.output.push(' ');
        self.write_block(&function.body, indent);
    }

    fn write_foreign_module(&mut self, module: &ForeignModuleDecl, indent: usize) {
        self.output.push_str("foreign module ");
        self.output.push_str(&module.name);
        if module.functions.is_empty() {
            self.write_empty_or_commented_body(module.span.end, indent);
            return;
        }

        self.output.push_str(" {\n");
        for (index, function) in module.functions.iter().enumerate() {
            self.write_comments_before(function.span.start, indent + 1);
            self.ensure_indent(indent + 1);
            self.output.push_str("fn ");
            self.output.push_str(&function.name);
            self.output.push('(');
            for (parameter_index, parameter) in function.parameters.iter().enumerate() {
                if parameter_index > 0 {
                    self.output.push_str(", ");
                }
                self.output.push_str(&parameter.name);
                self.output.push_str(": ");
                self.write_type(&parameter.ty);
            }
            self.output.push(')');
            self.write_return_and_errors(&function.return_type, &function.error_set);

            let next_start = module
                .functions
                .get(index + 1)
                .map_or(module.span.end, |next| next.span.start);
            self.write_trailing_comment(function.span.end, next_start);
            self.output.push('\n');
        }
        self.write_comments_before(module.span.end, indent + 1);
        self.write_indent(indent);
        self.output.push('}');
    }

    fn write_script_block(&mut self, script: &ScriptBlock, indent: usize) {
        let base_pad = "    ".repeat(indent);
        let inner_pad = "    ".repeat(indent + 1);
        let exports_pad = "    ".repeat(indent + 2);

        self.output.push_str(&format!(
            "{{@script:{} as {};\n",
            script.language, script.name
        ));
        self.output
            .push_str(&format!("{inner_pad}@mode({})\n", script.mode));
        self.output.push_str(&format!("{inner_pad}@exports {{\n"));
        for export in &script.exports {
            self.output
                .push_str(&format!("{exports_pad}fn {}(", export.name));
            for (index, parameter) in export.parameters.iter().enumerate() {
                if index > 0 {
                    self.output.push_str(", ");
                }
                self.output.push_str(&parameter.name);
                self.output.push_str(": ");
                self.write_type(&parameter.ty);
            }
            self.output.push(')');
            self.write_return_and_errors(&export.return_type, &export.error_set);
            self.output.push('\n');
        }
        self.output.push_str(&format!("{inner_pad}}}\n\n"));
        self.output.push_str(&format!("{inner_pad}@impl\n"));
        self.output.push_str(&script.raw_source);
        if !script.raw_source.ends_with('\n') {
            self.output.push('\n');
        }
        self.output.push_str(&format!("{base_pad}@end\n"));
        self.output.push_str(&format!("{base_pad}}}"));
    }

    fn write_struct(&mut self, declaration: &StructDecl, indent: usize) {
        self.output.push_str("struct ");
        self.output.push_str(&declaration.name);
        self.write_type_parameters(&declaration.type_parameters);

        match &declaration.fields {
            StructFields::Tuple(fields) => {
                self.output.push('(');
                for (index, field) in fields.iter().enumerate() {
                    if index > 0 {
                        self.output.push_str(", ");
                    }
                    self.write_type(field);
                }
                self.output.push(')');
            }
            StructFields::Named(fields) => {
                if fields.is_empty() {
                    self.write_empty_or_commented_body(declaration.span.end, indent);
                    return;
                }

                self.output.push_str(" {\n");
                for (index, field) in fields.iter().enumerate() {
                    self.write_comments_before(field.span.start, indent + 1);
                    self.ensure_indent(indent + 1);
                    self.output.push_str(&field.name);
                    self.output.push_str(": ");
                    self.write_type(&field.ty);

                    let next_start = fields
                        .get(index + 1)
                        .map_or(declaration.span.end, |next| next.span.start);
                    self.write_trailing_comment(field.span.end, next_start);
                    self.output.push('\n');
                }
                self.write_comments_before(declaration.span.end, indent + 1);
                self.write_indent(indent);
                self.output.push('}');
            }
        }
    }

    fn write_enum(&mut self, declaration: &EnumDecl, indent: usize) {
        self.output.push_str("enum ");
        self.output.push_str(&declaration.name);
        self.write_type_parameters(&declaration.type_parameters);

        if declaration.variants.is_empty() {
            self.write_empty_or_commented_body(declaration.span.end, indent);
            return;
        }

        self.output.push_str(" {\n");
        for (index, variant) in declaration.variants.iter().enumerate() {
            self.write_comments_before(variant.span.start, indent + 1);
            self.ensure_indent(indent + 1);
            self.output.push_str(&variant.name);
            if let Some(fields) = &variant.fields {
                self.output.push('(');
                for (field_index, field) in fields.iter().enumerate() {
                    if field_index > 0 {
                        self.output.push_str(", ");
                    }
                    if let Some(name) = &field.name {
                        self.output.push_str(name);
                        self.output.push_str(": ");
                    }
                    self.write_type(&field.ty);
                }
                self.output.push(')');
            }

            let next_start = declaration
                .variants
                .get(index + 1)
                .map_or(declaration.span.end, |next| next.span.start);
            self.write_trailing_comment(variant.span.end, next_start);
            self.output.push('\n');
        }
        self.write_comments_before(declaration.span.end, indent + 1);
        self.write_indent(indent);
        self.output.push('}');
    }

    fn write_constant(&mut self, declaration: &ConstantDecl) {
        self.output.push_str("const ");
        self.output.push_str(&declaration.name);
        self.output.push_str(": ");
        self.write_type(&declaration.ty);
        self.output.push_str(" = ");
        self.write_expr(&declaration.value, 0);
    }

    fn write_type_alias(&mut self, declaration: &TypeAliasDecl) {
        self.output.push_str("type ");
        self.output.push_str(&declaration.name);
        self.write_type_parameters(&declaration.type_parameters);
        self.output.push_str(" = ");
        self.write_type(&declaration.ty);
    }

    fn write_block(&mut self, block: &Block, indent: usize) {
        if block.statements.is_empty() {
            if self.has_comment_before(block.span.end) {
                self.output.push_str("{\n");
                self.write_comments_before(block.span.end, indent + 1);
                self.write_indent(indent);
                self.output.push('}');
            } else {
                self.output.push_str("{}");
            }
            return;
        }

        self.output.push_str("{\n");
        self.write_statements(&block.statements, indent + 1, block.span.end);
        self.write_indent(indent);
        self.output.push('}');
    }

    fn write_statements(&mut self, statements: &[Stmt], indent: usize, end: usize) {
        for (index, statement) in statements.iter().enumerate() {
            self.write_comments_before(statement.span.start, indent);
            self.ensure_indent(indent);
            self.write_statement(statement, indent);

            let next_start = statements
                .get(index + 1)
                .map_or(end, |next| next.span.start);
            let trailing = self.write_trailing_comment(statement.span.end, next_start);

            if index + 1 == statements.len() || trailing {
                self.output.push('\n');
            } else if self.same_line(statement.span.end, statements[index + 1].span.start) {
                self.output.push_str("; ");
            } else {
                self.output.push('\n');
            }
        }

        self.write_comments_before(end, indent);
    }

    fn write_statement(&mut self, statement: &Stmt, indent: usize) {
        let previous_indent = self.current_indent;
        self.current_indent = indent;

        match &statement.kind {
            StmtKind::Let {
                mutable,
                name,
                ty,
                value,
            } => {
                self.output.push_str("let ");
                if *mutable {
                    self.output.push_str("mut ");
                }
                self.output.push_str(name);
                if let Some(ty) = ty {
                    self.output.push_str(": ");
                    self.write_type(ty);
                }
                self.output.push_str(" = ");
                self.write_expr(value, 0);
            }
            StmtKind::Return { value } => {
                self.output.push_str("return");
                if let Some(value) = value {
                    self.output.push(' ');
                    self.write_expr(value, 0);
                }
            }
            StmtKind::Break => self.output.push_str("break"),
            StmtKind::Continue => self.output.push_str("continue"),
            StmtKind::Defer(statement) => {
                self.output.push_str("defer ");
                self.write_statement_inline(statement);
            }
            StmtKind::While { condition, body } => {
                self.output.push_str("while ");
                self.write_expr(condition, 0);
                self.output.push(' ');
                self.write_block(body, indent);
            }
            StmtKind::For {
                binding,
                iterable,
                body,
            } => {
                self.output.push_str("for ");
                self.output.push_str(binding);
                self.output.push_str(" in ");
                self.write_expr(iterable, 0);
                self.output.push(' ');
                self.write_block(body, indent);
            }
            StmtKind::Expr(expression) => self.write_expr(expression, 0),
            StmtKind::Scoped(scoped) => self.write_scoped_statements(scoped, indent),
        }

        self.current_indent = previous_indent;
    }

    fn write_statement_inline(&mut self, statement: &Stmt) {
        match &statement.kind {
            StmtKind::Break => self.output.push_str("break"),
            StmtKind::Continue => self.output.push_str("continue"),
            StmtKind::Expr(expression) => self.write_expr(expression, 0),
            StmtKind::Return { value } => {
                self.output.push_str("return");
                if let Some(value) = value {
                    self.output.push(' ');
                    self.write_expr(value, 0);
                }
            }
            _ => {
                let before = self.output.len();
                let indent = self.current_indent;
                self.write_statement(statement, indent);
                if self.output.len() == before {
                    self.output.push_str("{ ... }");
                }
            }
        }
    }

    fn write_scoped_items(&mut self, scoped: &ScopedItems, indent: usize) {
        self.write_scope_annotation(&scoped.annotation);
        if scoped.items.is_empty() {
            if self.has_comment_before(scoped.span.end) {
                self.output.push('\n');
                self.write_comments_before(scoped.span.end, indent + 1);
                self.write_indent(indent);
            }
            self.output.push('}');
            return;
        }
        self.output.push('\n');
        self.write_items(&scoped.items, indent + 1, false, scoped.span.end);
        self.write_indent(indent);
        self.output.push('}');
    }

    fn write_scoped_statements(&mut self, scoped: &ScopedStatements, indent: usize) {
        self.write_scope_annotation(&scoped.annotation);
        if scoped.statements.is_empty() {
            if self.has_comment_before(scoped.span.end) {
                self.output.push('\n');
                self.write_comments_before(scoped.span.end, indent + 1);
                self.write_indent(indent);
            }
            self.output.push('}');
            return;
        }
        self.output.push('\n');
        self.write_statements(&scoped.statements, indent + 1, scoped.span.end);
        self.write_indent(indent);
        self.output.push('}');
    }

    fn write_scope_annotation(&mut self, annotation: &ScopeAnnotation) {
        self.output.push_str("{@");
        self.output.push_str(match annotation.kind {
            ScopeKind::Mode => "mode",
            ScopeKind::Memory => "memory",
        });
        self.output.push(':');
        self.output.push_str(&annotation.value);
        self.output.push(';');
    }

    fn write_import(&mut self, import: &ImportDecl) {
        self.output.push_str("import ");

        match &import.form {
            ImportForm::Path { path, alias } => {
                self.output.push_str(&path.join("."));
                if let Some(alias) = alias {
                    self.output.push_str(" as ");
                    self.output.push_str(alias);
                }
            }
            ImportForm::SideEffect { specifier } => {
                self.write_string(specifier);
            }
            ImportForm::Namespace { alias, specifier } => {
                self.output.push_str("* as ");
                self.output.push_str(alias);
                self.output.push_str(" from ");
                self.write_string(specifier);
            }
            ImportForm::Named {
                specifiers,
                specifier,
            } => {
                self.write_import_specifiers(specifiers);
                self.output.push_str(" from ");
                self.write_string(specifier);
            }
            ImportForm::Default { alias, specifier } => {
                self.output.push_str(alias);
                self.output.push_str(" from ");
                self.write_string(specifier);
            }
            ImportForm::DefaultNamed {
                default,
                specifiers,
                specifier,
            } => {
                self.output.push_str(default);
                self.output.push_str(", ");
                self.write_import_specifiers(specifiers);
                self.output.push_str(" from ");
                self.write_string(specifier);
            }
        }
    }

    fn write_import_specifiers(&mut self, specifiers: &[ImportSpecifier]) {
        self.output.push('{');
        if !specifiers.is_empty() {
            self.output.push(' ');
        }
        for (index, specifier) in specifiers.iter().enumerate() {
            if index > 0 {
                self.output.push_str(", ");
            }
            self.output.push_str(&specifier.imported);
            if specifier.imported != specifier.local {
                self.output.push_str(" as ");
                self.output.push_str(&specifier.local);
            }
        }
        if !specifiers.is_empty() {
            self.output.push(' ');
        }
        self.output.push('}');
    }

    fn write_export(&mut self, export: &ExportDecl, indent: usize) {
        match &export.form {
            ExportForm::List { specifiers, from } => {
                self.output.push_str("export ");
                self.write_export_specifiers(specifiers);
                if let Some(from) = from {
                    self.output.push_str(" from ");
                    self.write_string(from);
                }
            }
            ExportForm::All { alias, from } => {
                self.output.push_str("export *");
                if let Some(alias) = alias {
                    self.output.push_str(" as ");
                    self.output.push_str(alias);
                }
                self.output.push_str(" from ");
                self.write_string(from);
            }
            ExportForm::DefaultValue(expression) => {
                self.output.push_str("export default ");
                self.write_expr(expression, 0);
            }
            ExportForm::DefaultFunction(function) => {
                self.output.push_str("export default ");
                self.write_function(function, indent);
            }
        }
    }

    fn write_export_specifiers(&mut self, specifiers: &[ExportSpecifier]) {
        self.output.push('{');
        if !specifiers.is_empty() {
            self.output.push(' ');
        }
        for (index, specifier) in specifiers.iter().enumerate() {
            if index > 0 {
                self.output.push_str(", ");
            }
            self.output.push_str(&specifier.local);
            if specifier.local != specifier.exported {
                self.output.push_str(" as ");
                self.output.push_str(&specifier.exported);
            }
        }
        if !specifiers.is_empty() {
            self.output.push(' ');
        }
        self.output.push('}');
    }

    fn write_attribute(&mut self, attribute: &Attribute, _indent: usize) {
        self.output.push('@');
        self.output.push_str(&attribute.name);
        if let Some(spec) = &attribute.export_as {
            self.output.push('(');
            self.write_literal(&Literal::String(spec.symbol.clone()));
            for (parameter, ty) in &spec.bindings {
                self.output.push_str(", ");
                self.output.push_str(parameter);
                self.output.push_str(" = ");
                self.write_type(ty);
            }
            self.output.push(')');
            return;
        }
        if !attribute.arguments.is_empty() {
            self.output.push('(');
            for (index, argument) in attribute.arguments.iter().enumerate() {
                if index > 0 {
                    self.output.push_str(", ");
                }
                self.write_expr(argument, 0);
            }
            self.output.push(')');
        }
    }

    fn write_expr(&mut self, expression: &Expr, parent_precedence: u8) {
        let precedence = expression_precedence(&expression.kind);
        let needs_parentheses = precedence < parent_precedence;

        if needs_parentheses {
            self.output.push('(');
        }

        match &expression.kind {
            ExprKind::Literal(literal) => self.write_literal(literal),
            ExprKind::Identifier(identifier) => self.output.push_str(identifier),
            ExprKind::Unary { operator, operand } => {
                match operator {
                    UnaryOp::Negate => self.output.push('-'),
                    UnaryOp::Not => self.output.push_str("not "),
                }
                self.write_expr(operand, precedence);
            }
            ExprKind::Binary {
                operator,
                left,
                right,
            } => {
                self.write_expr(left, precedence);
                self.output.push(' ');
                self.output.push_str(binary_operator(*operator));
                self.output.push(' ');
                self.write_expr(right, precedence + 1);
            }
            ExprKind::Assign { target, value } => {
                self.write_expr(target, precedence);
                self.output.push_str(" = ");
                self.write_expr(value, precedence);
            }
            ExprKind::Call { callee, arguments } => {
                self.write_expr(callee, precedence);
                self.output.push('(');
                for (index, argument) in arguments.iter().enumerate() {
                    if index > 0 {
                        self.output.push_str(", ");
                    }
                    self.write_expr(argument, 0);
                }
                self.output.push(')');
            }
            ExprKind::Index { target, index } => {
                self.write_expr(target, precedence);
                self.output.push('[');
                self.write_expr(index, 0);
                self.output.push(']');
            }
            ExprKind::Member { target, member } => {
                self.write_expr(target, precedence);
                self.output.push('.');
                self.output.push_str(member);
            }
            ExprKind::TypeArguments { target, arguments } => {
                self.write_expr(target, precedence);
                self.write_type_arguments(arguments);
            }
            ExprKind::Propagate(target) => {
                self.write_expr(target, precedence);
                self.output.push('!');
            }
            ExprKind::Array(elements) => {
                self.output.push('[');
                for (index, element) in elements.iter().enumerate() {
                    if index > 0 {
                        self.output.push_str(", ");
                    }
                    self.write_expr(element, 0);
                }
                self.output.push(']');
            }
            ExprKind::Map(entries) => {
                self.output.push('{');
                for (index, entry) in entries.iter().enumerate() {
                    if index > 0 {
                        self.output.push_str(", ");
                    }
                    self.write_expr(&entry.key, 0);
                    self.output.push_str(": ");
                    self.write_expr(&entry.value, 0);
                }
                self.output.push('}');
            }
            ExprKind::Tuple(elements) => {
                self.output.push('(');
                for (index, element) in elements.iter().enumerate() {
                    if index > 0 {
                        self.output.push_str(", ");
                    }
                    self.write_expr(element, 0);
                }
                self.output.push(')');
            }
            ExprKind::StructInit { path, fields } => {
                self.write_type_path(path);
                if fields.is_empty() {
                    self.output.push_str(" {}");
                } else {
                    self.output.push_str(" { ");
                    for (index, field) in fields.iter().enumerate() {
                        if index > 0 {
                            self.output.push_str(", ");
                        }
                        self.output.push_str(&field.name);
                        self.output.push_str(": ");
                        self.write_expr(&field.value, 0);
                    }
                    self.output.push_str(" }");
                }
            }
            ExprKind::Block(block) => self.write_block(block, self.current_indent),
            ExprKind::If {
                condition,
                then_branch,
                else_branch,
            } => {
                self.output.push_str("if ");
                self.write_expr(condition, 0);
                self.output.push(' ');
                self.write_block(then_branch, self.current_indent);
                if let Some(else_branch) = else_branch {
                    self.output.push_str(" else ");
                    match else_branch {
                        ElseBranch::Block(block) => self.write_block(block, self.current_indent),
                        ElseBranch::If(expression) => self.write_expr(expression, 0),
                    }
                }
            }
            ExprKind::Match { value, arms } => {
                self.output.push_str("match ");
                self.write_expr(value, 0);
                let arm_indent = self.current_indent + 1;
                if arms.is_empty() {
                    if self.has_comment_before(expression.span.end) {
                        self.output.push_str(" {\n");
                        self.write_comments_before(expression.span.end, arm_indent);
                        self.write_indent(self.current_indent);
                        self.output.push('}');
                    } else {
                        self.output.push_str(" {}");
                    }
                } else {
                    self.output.push_str(" {\n");
                    for (index, arm) in arms.iter().enumerate() {
                        self.write_comments_before(arm.span.start, arm_indent);
                        self.write_indent(arm_indent);
                        self.write_pattern(&arm.pattern);
                        self.output.push_str(" => ");

                        let previous_indent = self.current_indent;
                        self.current_indent = arm_indent;
                        match &arm.body {
                            MatchArmBody::Expr(expression) => self.write_expr(expression, 0),
                            MatchArmBody::Block(block) => self.write_block(block, arm_indent),
                        }
                        self.current_indent = previous_indent;

                        let next_start = arms
                            .get(index + 1)
                            .map_or(expression.span.end, |next| next.span.start);
                        self.write_trailing_comment(arm.span.end, next_start);
                        self.output.push('\n');
                    }
                    self.write_comments_before(expression.span.end, arm_indent);
                    self.write_indent(self.current_indent);
                    self.output.push('}');
                }
            }
            ExprKind::Spawn { body } => {
                self.output.push_str("spawn || ");
                self.write_block(body, self.current_indent);
            }
            ExprKind::Fail { error, arguments } => {
                self.output.push('!');
                self.write_type_path(error);
                match arguments {
                    FailArguments::Tuple(arguments) => {
                        self.output.push('(');
                        for (index, argument) in arguments.iter().enumerate() {
                            if index > 0 {
                                self.output.push_str(", ");
                            }
                            self.write_expr(argument, 0);
                        }
                        self.output.push(')');
                    }
                    FailArguments::Struct(fields) => {
                        self.output.push_str(" { ");
                        for (index, field) in fields.iter().enumerate() {
                            if index > 0 {
                                self.output.push_str(", ");
                            }
                            self.output.push_str(&field.name);
                            self.output.push_str(": ");
                            self.write_expr(&field.value, 0);
                        }
                        self.output.push_str(" }");
                    }
                }
            }
            ExprKind::Panic { message } => {
                self.output.push_str("panic(");
                self.write_expr(message, 0);
                self.output.push(')');
            }
            ExprKind::Abort { message } => {
                self.output.push_str("abort(");
                self.write_expr(message, 0);
                self.output.push(')');
            }
        }

        if needs_parentheses {
            self.output.push(')');
        }
    }

    fn write_literal(&mut self, literal: &Literal) {
        match literal {
            Literal::Integer(value) | Literal::Float(value) => self.output.push_str(value),
            Literal::String(value) => self.write_string(value),
            Literal::Bytes(value) => self.write_bytes(value),
            Literal::Bool(value) => self.output.push_str(if *value { "true" } else { "false" }),
            Literal::Unit => self.output.push_str("()"),
        }
    }

    fn write_string(&mut self, value: &str) {
        self.output.push('"');
        for character in value.chars() {
            match character {
                '\\' => self.output.push_str("\\\\"),
                '"' => self.output.push_str("\\\""),
                '\n' => self.output.push_str("\\n"),
                '\r' => self.output.push_str("\\r"),
                '\t' => self.output.push_str("\\t"),
                '\0' => self.output.push_str("\\0"),
                character if character.is_control() => {
                    self.output
                        .push_str(&format!("\\u{{{:X}}}", character as u32));
                }
                character => self.output.push(character),
            }
        }
        self.output.push('"');
    }

    fn write_bytes(&mut self, value: &[u8]) {
        self.output.push_str("b\"");
        for byte in value {
            match *byte {
                b'\\' => self.output.push_str("\\\\"),
                b'"' => self.output.push_str("\\\""),
                b'\n' => self.output.push_str("\\n"),
                b'\r' => self.output.push_str("\\r"),
                b'\t' => self.output.push_str("\\t"),
                0 => self.output.push_str("\\0"),
                0x20..=0x7E => self.output.push(char::from(*byte)),
                byte => self.output.push_str(&format!("\\u{{{byte:02X}}}")),
            }
        }
        self.output.push('"');
    }

    fn write_pattern(&mut self, pattern: &Pattern) {
        match &pattern.kind {
            PatternKind::Wildcard => self.output.push('_'),
            PatternKind::Identifier(identifier) => self.output.push_str(identifier),
            PatternKind::Literal(literal) => self.write_literal(literal),
            PatternKind::Tuple(patterns) => {
                self.output.push('(');
                for (index, pattern) in patterns.iter().enumerate() {
                    if index > 0 {
                        self.output.push_str(", ");
                    }
                    self.write_pattern(pattern);
                }
                self.output.push(')');
            }
            PatternKind::Constructor { path, arguments } => {
                self.write_type_path(path);
                self.output.push('(');
                for (index, pattern) in arguments.iter().enumerate() {
                    if index > 0 {
                        self.output.push_str(", ");
                    }
                    self.write_pattern(pattern);
                }
                self.output.push(')');
            }
            PatternKind::Variant {
                path,
                variant,
                arguments,
            } => {
                self.write_type_path(path);
                self.output.push('.');
                self.output.push_str(variant);
                if let Some(arguments) = arguments {
                    self.output.push('(');
                    for (index, pattern) in arguments.iter().enumerate() {
                        if index > 0 {
                            self.output.push_str(", ");
                        }
                        self.write_pattern(pattern);
                    }
                    self.output.push(')');
                }
            }
        }
    }

    fn write_type(&mut self, ty: &Type) {
        match &ty.kind {
            TypeKind::Path(path) => self.write_type_path(path),
            TypeKind::Array(element) => {
                self.output.push('[');
                self.write_type(element);
                self.output.push(']');
            }
            TypeKind::FixedArray(element, length) => {
                self.output.push('[');
                self.write_type(element);
                self.output.push_str("; ");
                self.output.push_str(length);
                self.output.push(']');
            }
            TypeKind::Map(key, value) => {
                self.output.push('{');
                self.write_type(key);
                self.output.push_str(": ");
                self.write_type(value);
                self.output.push('}');
            }
            TypeKind::Tuple(elements) => {
                self.output.push('(');
                for (index, element) in elements.iter().enumerate() {
                    if index > 0 {
                        self.output.push_str(", ");
                    }
                    self.write_type(element);
                }
                self.output.push(')');
            }
            TypeKind::Function {
                parameters,
                return_type,
                error_set,
            } => {
                self.output.push_str("fn(");
                for (index, parameter) in parameters.iter().enumerate() {
                    if index > 0 {
                        self.output.push_str(", ");
                    }
                    self.write_type(parameter);
                }
                self.output.push(')');
                if let Some(return_type) = return_type {
                    self.output.push_str(" -> ");
                    self.write_type(return_type);
                    self.write_error_set(error_set);
                }
            }
            TypeKind::Unit => self.output.push_str("()"),
        }
    }

    fn write_type_path(&mut self, path: &TypePath) {
        self.output.push_str(&path.segments.join("."));
        self.write_type_arguments(&path.arguments);
    }

    fn write_type_parameters(&mut self, parameters: &[String]) {
        if parameters.is_empty() {
            return;
        }
        self.output.push('<');
        self.output.push_str(&parameters.join(", "));
        self.output.push('>');
    }

    fn write_type_arguments(&mut self, arguments: &[Type]) {
        if arguments.is_empty() {
            return;
        }
        self.output.push('<');
        for (index, argument) in arguments.iter().enumerate() {
            if index > 0 {
                self.output.push_str(", ");
            }
            self.write_type(argument);
        }
        self.output.push('>');
    }

    fn write_return_and_errors(&mut self, return_type: &Option<Type>, error_set: &[TypePath]) {
        if let Some(return_type) = return_type {
            self.output.push_str(" -> ");
            self.write_type(return_type);
            self.write_error_set(error_set);
        }
    }

    fn write_error_set(&mut self, error_set: &[TypePath]) {
        if error_set.is_empty() {
            return;
        }
        self.output.push_str(" ! ");
        for (index, error) in error_set.iter().enumerate() {
            if index > 0 {
                self.output.push_str(" | ");
            }
            self.write_type_path(error);
        }
    }

    fn write_empty_or_commented_body(&mut self, end: usize, indent: usize) {
        if self.has_comment_before(end) {
            self.output.push_str(" {\n");
            self.write_comments_before(end, indent + 1);
            self.write_indent(indent);
            self.output.push('}');
        } else {
            self.output.push_str(" {}");
        }
    }

    fn write_comments_before(&mut self, offset: usize, indent: usize) {
        while let Some(comment) = self.comments.get(self.comment_index) {
            if comment.span.start >= offset {
                break;
            }

            let consumed = self.consumed_comments[self.comment_index];
            self.comment_index += 1;
            if consumed {
                continue;
            }

            if !self.output.is_empty() && !self.output.ends_with('\n') {
                self.output.push('\n');
            }
            self.write_indent(indent);
            self.output.push_str(&comment.text);
            self.output.push('\n');
        }
    }

    fn write_trailing_comment(&mut self, line_end: usize, limit: usize) -> bool {
        let line = self.line_of(line_end);

        for index in self.comment_index..self.comments.len() {
            let comment = &self.comments[index];
            if comment.span.start >= limit {
                break;
            }
            if self.consumed_comments[index]
                || comment.span.start < line_end
                || self.line_of(comment.span.start) != line
            {
                continue;
            }

            self.output.push(' ');
            self.output.push_str(&comment.text);
            self.consumed_comments[index] = true;
            return true;
        }

        false
    }

    fn has_comment_before(&self, offset: usize) -> bool {
        self.comments
            .iter()
            .enumerate()
            .skip(self.comment_index)
            .any(|(index, comment)| !self.consumed_comments[index] && comment.span.start < offset)
    }

    fn ensure_indent(&mut self, indent: usize) {
        if self.output.is_empty() || self.output.ends_with('\n') {
            self.write_indent(indent);
        }
    }

    fn write_indent(&mut self, indent: usize) {
        for _ in 0..indent {
            self.output.push_str("    ");
        }
    }

    fn same_line(&self, left_end: usize, right_start: usize) -> bool {
        self.line_of(left_end) == self.line_of(right_start)
    }

    fn line_of(&self, offset: usize) -> usize {
        self.line_starts
            .partition_point(|line_start| *line_start <= offset)
            .saturating_sub(1)
    }
}

fn expression_precedence(kind: &ExprKind) -> u8 {
    match kind {
        ExprKind::Assign { .. } => 1,
        ExprKind::Binary { operator, .. } => match operator {
            BinaryOp::Or => 2,
            BinaryOp::And => 3,
            BinaryOp::Equal | BinaryOp::NotEqual => 4,
            BinaryOp::Less | BinaryOp::LessEqual | BinaryOp::Greater | BinaryOp::GreaterEqual => 5,
            BinaryOp::Add | BinaryOp::Subtract | BinaryOp::AddWrap | BinaryOp::SubtractWrap => 6,
            BinaryOp::Multiply
            | BinaryOp::Divide
            | BinaryOp::Remainder
            | BinaryOp::MultiplyWrap
            | BinaryOp::DivideWrap => 7,
        },
        ExprKind::Unary { .. } => 8,
        ExprKind::Call { .. }
        | ExprKind::Index { .. }
        | ExprKind::Member { .. }
        | ExprKind::TypeArguments { .. }
        | ExprKind::Propagate(_)
        | ExprKind::StructInit { .. } => 9,
        _ => 10,
    }
}

fn binary_operator(operator: BinaryOp) -> &'static str {
    match operator {
        BinaryOp::Add => "+",
        BinaryOp::Subtract => "-",
        BinaryOp::Multiply => "*",
        BinaryOp::Divide => "/",
        BinaryOp::Remainder => "%",
        BinaryOp::AddWrap => "+%",
        BinaryOp::SubtractWrap => "-%",
        BinaryOp::MultiplyWrap => "*%",
        BinaryOp::DivideWrap => "/%",
        BinaryOp::Equal => "==",
        BinaryOp::NotEqual => "!=",
        BinaryOp::Less => "<",
        BinaryOp::LessEqual => "<=",
        BinaryOp::Greater => ">",
        BinaryOp::GreaterEqual => ">=",
        BinaryOp::And => "&&",
        BinaryOp::Or => "||",
    }
}
