use crate::ast::*;
use crate::diagnostic::{Diagnostic, Severity};
use crate::edition::Edition;
use crate::lexer::{Keyword, LexOptions, Punctuation, TokenKind, lex_with_options};
use crate::parser::{ParseOptions, parse_with_comments_with_options};

const ALIASES: [(&str, &str); 7] = [
    ("function", "fn"),
    ("var", "let"),
    ("constant", "const"),
    ("typedef", "type"),
    ("record", "struct"),
    ("mutable", "mut"),
    ("public", "pub"),
];
const UNSAFE_DIVISION: &str = "NS2013";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MigrateOptions {
    pub target_edition: Edition,

    pub change_edition: bool,
}

impl Default for MigrateOptions {
    fn default() -> Self {
        Self {
            target_edition: Edition::V0_2,
            change_edition: false,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Migration {
    pub source: String,
    pub diagnostics: Vec<Diagnostic>,
    pub changed: bool,
}

struct Edit {
    start: usize,
    end: usize,
    replacement: String,
}

pub fn migrate_source(source: &str, options: MigrateOptions) -> Result<Migration, Vec<Diagnostic>> {
    if options.target_edition != Edition::V0_2 {
        return Err(vec![Diagnostic::error(
            "NS2008",
            "Migration currently targets edition 0.2 only.",
        )]);
    }

    let tokens = lex_with_options(
        source,
        LexOptions {
            edition: Edition::V0_1,
        },
    )
    .tokens;
    let mut edits = Vec::new();
    let mut diagnostics = Vec::new();

    for token in &tokens {
        if let Some((_, canonical)) = ALIASES
            .iter()
            .find(|(alias, _)| *alias == token.text.as_str())
        {
            edits.push(Edit {
                start: token.span.start,
                end: token.span.end,
                replacement: (*canonical).to_owned(),
            });
        }
        if token.kind == TokenKind::Punct(Punctuation::SlashWrap) {
            let mut warning = Diagnostic::error(
                UNSAFE_DIVISION,
                "Wrapping division '/%' is preserved during migration.",
            )
            .with_span(token.span);
            warning.severity = Severity::Warning;
            warning.hint = Some("Replacing it with '/' changes overflow semantics.".to_owned());
            diagnostics.push(warning);
        }
    }

    migrate_token_spellings(&tokens, &mut edits);

    let module = match parse_with_comments_with_options(
        source,
        ParseOptions {
            edition: Edition::V0_1,
        },
    ) {
        Ok((module, _)) => module,
        Err(_) => {
            parse_with_comments_with_options(
                source,
                ParseOptions {
                    edition: Edition::V0_2,
                },
            )?
            .0
        }
    };
    migrate_module(&module, source, &mut edits);

    edits.sort_by_key(|edit| std::cmp::Reverse(edit.start));
    let mut output = source.to_owned();
    let mut last_start = source.len() + 1;
    for edit in edits {
        if edit.start > edit.end || edit.end > last_start || edit.start > output.len() {
            continue;
        }
        output.replace_range(edit.start..edit.end, &edit.replacement);
        last_start = edit.start;
    }

    Ok(Migration {
        changed: output != source,
        source: output,
        diagnostics,
    })
}

fn next_significant(tokens: &[crate::lexer::Token], mut index: usize) -> Option<usize> {
    while tokens
        .get(index)
        .is_some_and(|token| token.kind == TokenKind::Newline)
    {
        index += 1;
    }
    tokens.get(index).map(|_| index)
}

fn migrate_token_spellings(tokens: &[crate::lexer::Token], edits: &mut Vec<Edit>) {
    for (index, token) in tokens.iter().enumerate() {
        if token.kind == TokenKind::Keyword(Keyword::Spawn)
            && let Some(pipe) = next_significant(tokens, index + 1)
            && tokens[pipe].kind == TokenKind::Punct(Punctuation::PipePipe)
        {
            edits.push(Edit {
                start: token.span.end,
                end: tokens[pipe].span.end,
                replacement: String::new(),
            });
        }

        if token.kind == TokenKind::Punct(Punctuation::LBrace)
            && let Some(at) = next_significant(tokens, index + 1)
            && tokens[at].kind == TokenKind::Punct(Punctuation::At)
            && let Some(name) = next_significant(tokens, at + 1)
            && tokens[name].kind == TokenKind::Identifier
            && let Some(colon) = next_significant(tokens, name + 1)
            && tokens[colon].kind == TokenKind::Punct(Punctuation::Colon)
            && let Some(value) = next_significant(tokens, colon + 1)
            && tokens[value].kind == TokenKind::Identifier
            && let Some(semicolon) = next_significant(tokens, value + 1)
            && tokens[semicolon].kind == TokenKind::Punct(Punctuation::Semicolon)
        {
            let kind = if tokens[name].text == "memory" {
                "memory"
            } else {
                "mode"
            };
            edits.push(Edit {
                start: token.span.start,
                end: tokens[semicolon].span.end,
                replacement: format!("@{kind}({}) {{", tokens[value].text),
            });
        }
    }
}

fn migrate_module(module: &Module, source: &str, edits: &mut Vec<Edit>) {
    for item in &module.items {
        migrate_item(item, source, edits);
    }
}

fn migrate_item(item: &Item, source: &str, edits: &mut Vec<Edit>) {
    match &item.kind {
        ItemKind::Function(function) => migrate_block(&function.body, source, edits),
        ItemKind::Constant(constant) => migrate_expr(&constant.value, source, edits),
        ItemKind::ScopedItems(scoped) => {
            for item in &scoped.items {
                migrate_item(item, source, edits);
            }
        }
        ItemKind::Export(export) => match &export.form {
            ExportForm::DefaultFunction(function) => migrate_block(&function.body, source, edits),
            ExportForm::DefaultValue(expression) => migrate_expr(expression, source, edits),
            _ => {}
        },
        ItemKind::ScriptBlock(_) => {}
        _ => {}
    }
}

fn migrate_block(block: &Block, source: &str, edits: &mut Vec<Edit>) {
    for statement in &block.statements {
        migrate_stmt(statement, source, edits);
    }
}

fn migrate_stmt(statement: &Stmt, source: &str, edits: &mut Vec<Edit>) {
    let redundant_return = match &statement.kind {
        StmtKind::Return { value: Some(value) } => {
            matches!(value.kind, ExprKind::Fail { .. })
                && source.as_bytes().get(value.span.start) == Some(&b'!')
        }
        _ => false,
    };
    match &statement.kind {
        StmtKind::Let { value, .. } => migrate_expr(value, source, edits),
        StmtKind::Return { value: Some(value) } => {
            if redundant_return {
                edits.push(Edit {
                    start: statement.span.start,
                    end: value.span.start,
                    replacement: String::new(),
                });
            }
            migrate_expr(value, source, edits);
        }
        StmtKind::Return { value: None } => {}
        StmtKind::Expr(expression) => migrate_expr(expression, source, edits),
        StmtKind::Defer(inner) => migrate_stmt(inner, source, edits),
        StmtKind::While { condition, body } => {
            migrate_expr(condition, source, edits);
            migrate_block(body, source, edits);
        }
        StmtKind::For { iterable, body, .. } => {
            migrate_expr(iterable, source, edits);
            migrate_block(body, source, edits);
        }
        StmtKind::Scoped(scoped) => {
            for statement in &scoped.statements {
                migrate_stmt(statement, source, edits);
            }
        }
        _ => {}
    }
}

fn migrate_expr(expression: &Expr, source: &str, edits: &mut Vec<Edit>) {
    match &expression.kind {
        ExprKind::Literal(_) | ExprKind::Identifier(_) => {}
        ExprKind::Unary { operand, .. } => migrate_expr(operand, source, edits),
        ExprKind::Binary { left, right, .. }
        | ExprKind::Assign {
            target: left,
            value: right,
        } => {
            migrate_expr(left, source, edits);
            migrate_expr(right, source, edits);
        }
        ExprKind::Call { callee, arguments } => {
            migrate_expr(callee, source, edits);
            for argument in arguments {
                migrate_expr(argument, source, edits);
            }
        }
        ExprKind::Index { target, index } => {
            migrate_expr(target, source, edits);
            migrate_expr(index, source, edits);
        }
        ExprKind::Member { target, .. } => migrate_expr(target, source, edits),
        ExprKind::TypeArguments { target, .. } => {
            let text = &source[expression.span.start..expression.span.end];
            if !text.contains("::")
                && let Some(position) = text.find('<')
            {
                edits.push(Edit {
                    start: expression.span.start + position,
                    end: expression.span.start + position,
                    replacement: "::".to_owned(),
                });
            }
            migrate_expr(target, source, edits);
        }
        ExprKind::Propagate(inner) => {
            let last = expression.span.end.saturating_sub(1);
            if source.as_bytes().get(last) == Some(&b'!') {
                edits.push(Edit {
                    start: last,
                    end: expression.span.end,
                    replacement: "?".to_owned(),
                });
            }
            migrate_expr(inner, source, edits);
        }
        ExprKind::Array(elements) | ExprKind::Tuple(elements) => {
            for element in elements {
                migrate_expr(element, source, edits);
            }
        }
        ExprKind::Map(entries) => {
            for entry in entries {
                migrate_expr(&entry.key, source, edits);
                migrate_expr(&entry.value, source, edits);
            }
        }
        ExprKind::StructInit { fields, .. } => {
            for field in fields {
                migrate_expr(&field.value, source, edits);
            }
        }
        ExprKind::Block(block) => migrate_block(block, source, edits),
        ExprKind::If {
            condition,
            then_branch,
            else_branch,
        } => {
            migrate_expr(condition, source, edits);
            migrate_block(then_branch, source, edits);
            if let Some(ElseBranch::Block(block)) = else_branch {
                migrate_block(block, source, edits);
            } else if let Some(ElseBranch::If(expression)) = else_branch {
                migrate_expr(expression, source, edits);
            }
        }
        ExprKind::Match { value, arms } => {
            migrate_expr(value, source, edits);
            for arm in arms {
                match &arm.body {
                    MatchArmBody::Expr(expression) => migrate_expr(expression, source, edits),
                    MatchArmBody::Block(block) => migrate_block(block, source, edits),
                }
            }
        }
        ExprKind::Spawn { body } => migrate_block(body, source, edits),
        ExprKind::Fail { arguments, .. } => {
            if source.as_bytes().get(expression.span.start) == Some(&b'!') {
                edits.push(Edit {
                    start: expression.span.start,
                    end: expression.span.start + 1,
                    replacement: "fail ".to_owned(),
                });
            }
            match arguments {
                FailArguments::Tuple(arguments) => {
                    for argument in arguments {
                        migrate_expr(argument, source, edits);
                    }
                }
                FailArguments::Struct(fields) => {
                    for field in fields {
                        migrate_expr(&field.value, source, edits);
                    }
                }
            }
        }
        ExprKind::Panic { message } | ExprKind::Abort { message } => {
            migrate_expr(message, source, edits)
        }
    }
}
