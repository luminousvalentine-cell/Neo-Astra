use crate::ast::*;
use crate::diagnostic::Diagnostic;
use crate::edition::Edition;
use crate::lexer::{
    Comment, Keyword, LexOptions, LexResult, Punctuation, Token, TokenKind, lex_with_options,
};
use crate::literal::{decode_byte_literal, decode_char_literal, decode_string_literal};
use crate::span::Span;

pub type ParseResult<T> = Result<T, Vec<Diagnostic>>;

const EXPECTED: &str = "NS2001";
const INVALID: &str = "NS2002";
const INVALID_TYPE: &str = "NS2003";
const INVALID_PATTERN: &str = "NS2004";
const INVALID_ITEM: &str = "NS2005";
const BOUNDARY: &str = "NS2006";
const BARE_TYPE_ARGUMENTS: &str = "NS2010";
const MALFORMED_TURBOFISH: &str = "NS2011";
const INVALID_TYPE_ARGUMENT: &str = "NS4018";
const DEPRECATED_SYNTAX: &str = "NS2015";
const INVALID_CONTROL_FLOW: &str = "NS4014";

const MAX_NESTING_DEPTH: usize = 64;
const CONFLICTING_VISIBILITY: &str = "NS3002";
const UNKNOWN_SCRIPT_LANGUAGE: &str = "NS7001";
const INVALID_SCRIPT_EXPORTS: &str = "NS7003";
const MALFORMED_SCRIPT_BLOCK: &str = "NS7004";
const INVALID_SCRIPT_SIGNATURE: &str = "NS7005";

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ParseOptions {
    pub edition: Edition,
}

pub fn parse(source: &str) -> ParseResult<Module> {
    parse_with_options(source, ParseOptions::default())
}

pub fn parse_with_options(source: &str, options: ParseOptions) -> ParseResult<Module> {
    parse_with_comments_with_options(source, options).map(|(module, _)| module)
}

pub fn parse_with_comments(source: &str) -> ParseResult<(Module, Vec<Comment>)> {
    parse_with_comments_with_options(source, ParseOptions::default())
}

pub fn parse_with_comments_with_options(
    source: &str,
    options: ParseOptions,
) -> ParseResult<(Module, Vec<Comment>)> {
    let LexResult {
        tokens,
        comments,
        diagnostics,
    } = lex_with_options(
        source,
        LexOptions {
            edition: options.edition,
        },
    );
    let mut parser = Parser {
        tokens,
        index: 0,
        diagnostics,
        pending_lambda: None,
        lambda_depth: 0,
        edition: options.edition,
        depth: 0,
        depth_reported: false,
        nesting_drained: false,
    };
    let module = parser.parse_module();

    if parser.diagnostics.is_empty() {
        Ok((module, comments))
    } else {
        Err(suppress_cascades(parser.diagnostics, source))
    }
}

fn suppress_cascades(diagnostics: Vec<Diagnostic>, source: &str) -> Vec<Diagnostic> {
    let mut per_line: std::collections::HashMap<usize, (usize, std::collections::HashSet<String>)> =
        std::collections::HashMap::new();
    diagnostics
        .into_iter()
        .filter(|diagnostic| {
            if !diagnostic.code.starts_with("NS2") || diagnostic.code == "NS2002" {
                return true;
            }
            let line = diagnostic.span.map_or(0, |span| {
                source.as_bytes()[..span.start.min(source.len())]
                    .iter()
                    .filter(|byte| **byte == b'\n')
                    .count()
            });
            let (count, codes) = per_line
                .entry(line)
                .or_insert((0, std::collections::HashSet::new()));
            if codes.contains(&diagnostic.code) {
                return false;
            }
            if *count >= 2 {
                return false;
            }
            codes.insert(diagnostic.code.clone());
            *count += 1;
            true
        })
        .collect()
}

struct PendingLambda {
    parameters: Vec<LambdaParameter>,
    body: Box<Expr>,
    span: Span,
}

struct LambdaParameter {
    name: String,
    ty: Option<Type>,
    span: Span,
}

struct Parser {
    tokens: Vec<Token>,
    index: usize,
    diagnostics: Vec<Diagnostic>,

    pending_lambda: Option<PendingLambda>,

    lambda_depth: usize,

    #[allow(dead_code)]
    edition: Edition,

    depth: usize,

    depth_reported: bool,

    nesting_drained: bool,
}

impl Parser {
    fn parse_module(&mut self) -> Module {
        let start = self.peek().span.start;
        let mut module = Module {
            span: Span::new(start, start),
            ..Module::default()
        };

        self.skip_separators();

        while !self.at_eof() {
            let before = self.index;
            let attributes = self.parse_attributes();

            if self.at_keyword(Keyword::Module) {
                let token = self.bump();
                module.name = self.parse_path();
                module.attributes = attributes;
                module.header_span = Span::new(token.span.start, self.last_end());
                self.finish_boundary(false);
                continue;
            }

            if let Some(item) = self.parse_item_with_attributes(attributes) {
                module.items.push(item);
            }

            self.finish_boundary(false);
            if self.index == before {
                self.bump();
            }
        }

        module.span = Span::new(start, self.last_end());
        module
    }

    fn parse_item_with_attributes(&mut self, attributes: Vec<Attribute>) -> Option<Item> {
        let start = self.peek().span.start;

        if self.at_punct(Punctuation::LBrace) && self.looks_like_script_block() {
            let script = self.parse_script_block();
            return Some(Item {
                attributes,
                visibility: Visibility::Normal,
                exported: false,
                kind: ItemKind::ScriptBlock(script.clone()),
                span: script.span,
            });
        }

        if self.at_canonical_scope() {
            let scoped = self.parse_canonical_scoped_items();
            return Some(Item {
                attributes,
                visibility: Visibility::Normal,
                exported: false,
                kind: ItemKind::ScopedItems(scoped.clone()),
                span: scoped.span,
            });
        }

        if self.at_punct(Punctuation::LBrace) && self.looks_like_scope_annotation() {
            let scoped = self.parse_scoped_items();
            return Some(Item {
                attributes,
                visibility: Visibility::Normal,
                exported: false,
                kind: ItemKind::ScopedItems(scoped.clone()),
                span: scoped.span,
            });
        }

        if self.at_keyword(Keyword::Import) {
            let import = self.parse_import();
            return Some(Item {
                attributes,
                visibility: Visibility::Normal,
                exported: false,
                kind: ItemKind::Import(import.clone()),
                span: import.span,
            });
        }

        if self.at_keyword(Keyword::Export) && self.export_is_declaration() {
            let export = self.parse_export();
            return Some(Item {
                attributes,
                visibility: Visibility::Normal,
                exported: false,
                kind: ItemKind::Export(export.clone()),
                span: export.span,
            });
        }

        self.parse_declaration_item(attributes, start)
    }

    fn parse_declaration_item(&mut self, attributes: Vec<Attribute>, start: usize) -> Option<Item> {
        let mut visibility = Visibility::Normal;
        let mut exported = false;
        let mut see_public = false;
        let mut see_private = false;
        let mut see_export = false;

        loop {
            if self.at_keyword(Keyword::Pub) {
                let token = self.bump();
                if see_public || see_private {
                    self.error(
                        CONFLICTING_VISIBILITY,
                        "Duplicate or conflicting visibility modifier.",
                        token.span,
                        "Use at most one of pub/public or private.",
                    );
                }
                see_public = true;
                visibility = Visibility::Public;
            } else if self.at_keyword(Keyword::Private) {
                let token = self.bump();
                if see_public || see_private {
                    self.error(
                        CONFLICTING_VISIBILITY,
                        "Duplicate or conflicting visibility modifier.",
                        token.span,
                        "Use at most one of pub/public or private.",
                    );
                }
                see_private = true;
                visibility = Visibility::Private;
            } else if self.at_keyword(Keyword::Export) {
                let token = self.bump();
                if see_export {
                    self.error(
                        CONFLICTING_VISIBILITY,
                        "Duplicate export modifier.",
                        token.span,
                        "Use a single export modifier.",
                    );
                }
                see_export = true;
                exported = true;
            } else {
                break;
            }
        }

        if see_private && (see_public || see_export) {
            self.error(
                CONFLICTING_VISIBILITY,
                "A private declaration cannot also be public or exported.",
                Span::new(start, self.last_end()),
                "Remove pub/export, or make the declaration non-private.",
            );
        }

        let kind = self.parse_declaration_kind()?;
        let span = Span::new(start, self.last_end());

        Some(Item {
            attributes,
            visibility,
            exported,
            kind,
            span,
        })
    }

    fn parse_declaration_kind(&mut self) -> Option<ItemKind> {
        if self.at_keyword(Keyword::Fn) {
            return Some(ItemKind::Function(self.parse_function_decl()));
        }

        if self.at_keyword(Keyword::Struct) {
            return Some(ItemKind::Struct(self.parse_struct_decl()));
        }

        if self.at_keyword(Keyword::Enum) {
            return Some(ItemKind::Enum(self.parse_enum_decl()));
        }

        if self.at_keyword(Keyword::Const) {
            return Some(ItemKind::Constant(self.parse_const_decl()));
        }

        if self.at_keyword(Keyword::Type) {
            return Some(ItemKind::TypeAlias(self.parse_type_alias_decl()));
        }

        if self.at_keyword(Keyword::Foreign) {
            return Some(ItemKind::ForeignModule(self.parse_foreign_module()));
        }

        self.error_current(
            INVALID_ITEM,
            "Expected a declaration.",
            "Use fn, struct, enum, const, type, foreign module, import, or export.",
        );

        if !self.at_eof()
            && !self.at(TokenKind::Newline)
            && !self.at_punct(Punctuation::Semicolon)
            && !self.at_punct(Punctuation::RBrace)
        {
            self.bump();
        }

        None
    }

    fn parse_attributes(&mut self) -> Vec<Attribute> {
        let mut attributes = Vec::new();

        while self.at_punct(Punctuation::At) {
            if self.at_canonical_scope() {
                break;
            }
            let start = self.bump().span.start;
            let name = self.expect_identifier().unwrap_or_default();
            let mut arguments = Vec::new();

            if self.eat_punct(Punctuation::LParen) {
                self.skip_newlines();

                if name == "export_as" {
                    let symbol_expr = self.parse_expression();
                    let symbol = match &symbol_expr.kind {
                        ExprKind::Literal(Literal::String(value)) => value.clone(),
                        _ => {
                            self.error(
                                EXPECTED,
                                "The first argument of '@export_as' must be a string literal naming the exported symbol.",
                                symbol_expr.span,
                                "Write @export_as(\"ns_module_fn_type\", T = i32).",
                            );
                            String::new()
                        }
                    };
                    let mut bindings = Vec::new();
                    loop {
                        if !self.eat_punct(Punctuation::Comma) {
                            break;
                        }
                        self.skip_newlines();
                        if self.at_punct(Punctuation::RParen) {
                            break;
                        }
                        let Some(parameter) = self.expect_identifier() else {
                            break;
                        };
                        self.expect_punct(Punctuation::Eq, "'='");
                        let ty = self.parse_type();
                        bindings.push((parameter, ty));
                    }
                    self.expect_punct(Punctuation::RParen, "')'");
                    let span = self.span_from(start);
                    attributes.push(Attribute {
                        name,
                        arguments: Vec::new(),
                        export_as: Some(ExportAsAttribute {
                            symbol,
                            bindings,
                            span,
                        }),
                        span,
                    });
                    self.skip_newlines();
                    continue;
                }

                if !self.at_punct(Punctuation::RParen) {
                    loop {
                        arguments.push(self.parse_expression());
                        if self.eat_punct(Punctuation::Comma) {
                            self.skip_newlines();
                            if self.at_punct(Punctuation::RParen) {
                                break;
                            }
                            continue;
                        }
                        break;
                    }
                }

                self.expect_punct(Punctuation::RParen, "')'");
            }

            attributes.push(Attribute {
                name,
                arguments,
                export_as: None,
                span: self.span_from(start),
            });
            self.skip_newlines();
        }

        attributes
    }

    fn parse_import(&mut self) -> ImportDecl {
        let start = self.bump().span.start;

        let form = if self.at(TokenKind::String) {
            ImportForm::SideEffect {
                specifier: self.expect_string(),
            }
        } else if self.eat_punct(Punctuation::Star) {
            self.expect_keyword(Keyword::As, "'as'");
            let alias = self.expect_identifier().unwrap_or_default();
            self.expect_keyword(Keyword::From, "'from'");
            let specifier = self.expect_string();
            ImportForm::Namespace { alias, specifier }
        } else if self.at_punct(Punctuation::LBrace) {
            let specifiers = self.parse_import_specifiers();
            self.expect_keyword(Keyword::From, "'from'");
            let specifier = self.expect_string();
            ImportForm::Named {
                specifiers,
                specifier,
            }
        } else if self.at(TokenKind::Identifier) {
            let first = self.bump().text;

            if self.eat_keyword(Keyword::From) {
                let specifier = self.expect_string();
                ImportForm::Default {
                    alias: first,
                    specifier,
                }
            } else if self.eat_punct(Punctuation::Comma) {
                let specifiers = self.parse_import_specifiers();
                self.expect_keyword(Keyword::From, "'from'");
                let specifier = self.expect_string();
                ImportForm::DefaultNamed {
                    default: first,
                    specifiers,
                    specifier,
                }
            } else {
                let mut path = vec![first];
                while self.eat_punct(Punctuation::Dot) {
                    path.push(self.expect_identifier().unwrap_or_default());
                }
                let alias = if self.eat_keyword(Keyword::As) {
                    Some(self.expect_identifier().unwrap_or_default())
                } else {
                    None
                };
                ImportForm::Path { path, alias }
            }
        } else {
            self.error_current(
                INVALID_ITEM,
                "Invalid import form.",
                "Use import path, import path as alias, or a string-specifier form.",
            );
            ImportForm::Path {
                path: Vec::new(),
                alias: None,
            }
        };

        ImportDecl {
            form,
            span: self.span_from(start),
        }
    }

    fn parse_import_specifiers(&mut self) -> Vec<ImportSpecifier> {
        let mut specifiers = Vec::new();
        self.expect_punct(Punctuation::LBrace, "'{'");
        self.skip_separators();

        while !self.at_punct(Punctuation::RBrace) && !self.at_eof() {
            let start = self.peek().span.start;
            let imported = self.parse_name_or_default();
            let local = if self.eat_keyword(Keyword::As) {
                self.expect_identifier().unwrap_or_default()
            } else {
                imported.clone()
            };

            specifiers.push(ImportSpecifier {
                imported,
                local,
                span: self.span_from(start),
            });

            self.skip_separators();
            if self.eat_punct(Punctuation::Comma) {
                self.skip_separators();
                continue;
            }
            if !self.at_punct(Punctuation::RBrace) {
                self.error_current(
                    EXPECTED,
                    "Expected ',' or '}' after import specifier.",
                    "Separate import names with commas.",
                );
                self.synchronize_until(&[Punctuation::Comma, Punctuation::RBrace]);
                self.eat_punct(Punctuation::Comma);
                self.skip_separators();
            }
        }

        self.expect_punct(Punctuation::RBrace, "'}'");
        specifiers
    }

    fn parse_export(&mut self) -> ExportDecl {
        let start = self.bump().span.start;

        let form = if self.at_punct(Punctuation::LBrace) {
            let specifiers = self.parse_export_specifiers();
            let from = if self.eat_keyword(Keyword::From) {
                Some(self.expect_string())
            } else {
                None
            };
            ExportForm::List { specifiers, from }
        } else if self.eat_punct(Punctuation::Star) {
            let alias = if self.eat_keyword(Keyword::As) {
                Some(self.parse_name_or_default())
            } else {
                None
            };
            self.expect_keyword(Keyword::From, "'from'");
            let from = self.expect_string();
            ExportForm::All { alias, from }
        } else if self.eat_keyword(Keyword::Default) {
            if self.at_keyword(Keyword::Fn) {
                ExportForm::DefaultFunction(self.parse_function_decl())
            } else {
                ExportForm::DefaultValue(self.parse_expression())
            }
        } else {
            self.error_current(
                INVALID_ITEM,
                "Invalid export form.",
                "Use export { ... }, export * from ..., or export default.",
            );
            ExportForm::List {
                specifiers: Vec::new(),
                from: None,
            }
        };

        ExportDecl {
            form,
            span: self.span_from(start),
        }
    }

    fn parse_export_specifiers(&mut self) -> Vec<ExportSpecifier> {
        let mut specifiers = Vec::new();
        self.expect_punct(Punctuation::LBrace, "'{'");
        self.skip_separators();

        while !self.at_punct(Punctuation::RBrace) && !self.at_eof() {
            let start = self.peek().span.start;
            let local = self.parse_name_or_default();
            let exported = if self.eat_keyword(Keyword::As) {
                self.parse_name_or_default()
            } else {
                local.clone()
            };

            specifiers.push(ExportSpecifier {
                local,
                exported,
                span: self.span_from(start),
            });

            self.skip_separators();
            if self.eat_punct(Punctuation::Comma) {
                self.skip_separators();
                continue;
            }
            if !self.at_punct(Punctuation::RBrace) {
                self.error_current(
                    EXPECTED,
                    "Expected ',' or '}' after export specifier.",
                    "Separate export names with commas.",
                );
                self.synchronize_until(&[Punctuation::Comma, Punctuation::RBrace]);
                self.eat_punct(Punctuation::Comma);
                self.skip_separators();
            }
        }

        self.expect_punct(Punctuation::RBrace, "'}'");
        specifiers
    }

    fn export_is_declaration(&self) -> bool {
        matches!(
            self.peek_at(1).map(|token| token.kind),
            Some(TokenKind::Punct(Punctuation::LBrace))
                | Some(TokenKind::Punct(Punctuation::Star))
                | Some(TokenKind::Keyword(Keyword::Default))
        )
    }

    fn parse_function_decl(&mut self) -> FunctionDecl {
        let start = self.bump().span.start;
        let name = self.expect_identifier().unwrap_or_default();
        let type_parameters = self.parse_type_parameters();
        let parameters = self.parse_parameters();
        let (return_type, error_set) = self.parse_return_and_errors();
        let body = self.parse_block();

        FunctionDecl {
            name,
            type_parameters,
            parameters,
            return_type,
            error_set,
            body,
            span: self.span_from(start),
        }
    }

    fn recover_to_anchor(&mut self, anchors: &[Punctuation]) {
        while !self.at_eof()
            && !self.at(TokenKind::Newline)
            && !anchors.iter().any(|anchor| self.at_punct(*anchor))
        {
            self.bump();
        }
    }

    fn parse_parameters(&mut self) -> Vec<Parameter> {
        let mut parameters = Vec::new();
        self.expect_punct(Punctuation::LParen, "'('");
        self.skip_newlines();

        if self.at_punct(Punctuation::RParen) {
            self.bump();
            return parameters;
        }

        let mut recovered = false;
        loop {
            let start = self.peek().span.start;
            if !self.at(TokenKind::Identifier) {
                self.error_current(
                    EXPECTED,
                    "Expected a parameter name.",
                    "Write name: Type inside the parentheses.",
                );
                self.recover_to_anchor(&[Punctuation::RParen, Punctuation::LBrace]);
                recovered = true;
                break;
            }
            let name = self.expect_identifier().unwrap_or_default();
            self.expect_punct(Punctuation::Colon, "':'");
            let ty = self.parse_type();
            parameters.push(Parameter {
                name,
                ty,
                span: self.span_from(start),
            });

            if self.eat_punct(Punctuation::Comma) {
                self.skip_newlines();
                if self.at_punct(Punctuation::RParen) {
                    break;
                }
                continue;
            }
            break;
        }

        self.skip_newlines();
        if recovered {
            self.eat_punct(Punctuation::RParen);
        } else {
            self.expect_punct(Punctuation::RParen, "')'");
        }
        parameters
    }

    fn parse_type_parameters(&mut self) -> Vec<String> {
        let mut parameters = Vec::new();
        if !self.eat_punct(Punctuation::Lt) {
            return parameters;
        }

        loop {
            parameters.push(self.expect_identifier().unwrap_or_default());
            if self.eat_punct(Punctuation::Comma) {
                self.skip_newlines();
                if self.edition == Edition::V0_2 && self.at_punct(Punctuation::Gt) {
                    break;
                }
                continue;
            }
            break;
        }

        self.skip_newlines();
        self.expect_punct(Punctuation::Gt, "'>'");
        parameters
    }

    fn parse_return_and_errors(&mut self) -> (Option<Type>, Vec<TypePath>) {
        let mut return_type = None;
        let mut error_set = Vec::new();

        if self.eat_punct(Punctuation::Arrow) {
            return_type = Some(self.parse_type());
            if self.at_punct(Punctuation::Bang) {
                error_set = self.parse_error_set();
            }
        } else if self.at_punct(Punctuation::Bang) {
            self.error_current(
                INVALID,
                "An error effect requires an explicit return type.",
                "Use '-> Type ! Error' instead of '! Error'.",
            );
            error_set = self.parse_error_set();
        }

        (return_type, error_set)
    }

    fn parse_error_set(&mut self) -> Vec<TypePath> {
        let mut errors = Vec::new();
        self.expect_punct(Punctuation::Bang, "'!'");

        if !self.at(TokenKind::Identifier) {
            self.error_current(
                EXPECTED,
                "Expected an error type after '!'.",
                "Write '! ErrorType' or '! First | Second'.",
            );
            return errors;
        }

        loop {
            errors.push(self.parse_type_path());
            if !self.eat_punct(Punctuation::Pipe) {
                break;
            }
        }

        errors
    }

    fn parse_struct_decl(&mut self) -> StructDecl {
        let start = self.bump().span.start;
        let name = self.expect_identifier().unwrap_or_default();
        let type_parameters = self.parse_type_parameters();

        let fields = if self.eat_punct(Punctuation::LBrace) {
            let mut fields = Vec::new();
            self.skip_newlines();

            while !self.at_punct(Punctuation::RBrace) && !self.at_eof() {
                let before = self.index;
                let field_start = self.peek().span.start;
                let field_name = self.expect_identifier().unwrap_or_default();
                self.expect_punct(Punctuation::Colon, "':'");
                let ty = self.parse_type();
                fields.push(FieldDecl {
                    name: field_name,
                    ty,
                    span: self.span_from(field_start),
                });

                if self.index == before {
                    self.bump();
                }
                self.skip_newlines();
                if self.eat_punct(Punctuation::Comma) {
                    self.skip_newlines();
                }
            }

            self.expect_punct(Punctuation::RBrace, "'}'");
            StructFields::Named(fields)
        } else if self.eat_punct(Punctuation::LParen) {
            StructFields::Tuple(self.parse_type_list_after_open())
        } else {
            self.error_current(
                EXPECTED,
                "Expected '{' or '(' after struct name.",
                "Declare named fields with { ... } or tuple fields with ( ... ).",
            );
            StructFields::Named(Vec::new())
        };

        StructDecl {
            name,
            type_parameters,
            fields,
            span: self.span_from(start),
        }
    }

    fn parse_enum_decl(&mut self) -> EnumDecl {
        let start = self.bump().span.start;
        let name = self.expect_identifier().unwrap_or_default();
        let type_parameters = self.parse_type_parameters();
        let mut variants = Vec::new();

        self.expect_punct(Punctuation::LBrace, "'{'");
        self.skip_newlines();

        while !self.at_punct(Punctuation::RBrace) && !self.at_eof() {
            let before = self.index;
            let variant_start = self.peek().span.start;
            let variant_name = self.expect_identifier().unwrap_or_default();
            let fields = if self.eat_punct(Punctuation::LParen) {
                let mut fields = Vec::new();
                self.skip_newlines();

                if !self.at_punct(Punctuation::RParen) {
                    loop {
                        fields.push(self.parse_variant_field());
                        self.skip_newlines();
                        if self.eat_punct(Punctuation::Comma) {
                            self.skip_newlines();
                            if self.at_punct(Punctuation::RParen) {
                                break;
                            }
                            continue;
                        }
                        break;
                    }
                }

                self.skip_newlines();
                self.expect_punct(Punctuation::RParen, "')'");
                Some(fields)
            } else {
                None
            };

            variants.push(EnumVariant {
                name: variant_name,
                fields,
                span: self.span_from(variant_start),
            });

            if self.index == before {
                self.bump();
            }
            self.skip_newlines();
            if self.eat_punct(Punctuation::Comma) {
                self.skip_newlines();
            }
        }

        self.expect_punct(Punctuation::RBrace, "'}'");

        EnumDecl {
            name,
            type_parameters,
            variants,
            span: self.span_from(start),
        }
    }

    fn parse_variant_field(&mut self) -> VariantField {
        let start = self.peek().span.start;
        let name = if self.at(TokenKind::Identifier)
            && self
                .peek_at(1)
                .is_some_and(|token| token.kind == TokenKind::Punct(Punctuation::Colon))
        {
            let name = self.bump().text;
            self.bump();
            Some(name)
        } else {
            None
        };

        VariantField {
            name,
            ty: self.parse_type(),
            span: self.span_from(start),
        }
    }

    fn parse_const_decl(&mut self) -> ConstantDecl {
        let start = self.bump().span.start;
        let name = self.expect_identifier().unwrap_or_default();
        self.expect_punct(Punctuation::Colon, "':'");
        let ty = self.parse_type();
        self.expect_punct(Punctuation::Eq, "'='");
        let value = self.parse_expression();

        ConstantDecl {
            name,
            ty,
            value,
            span: self.span_from(start),
        }
    }

    fn parse_type_alias_decl(&mut self) -> TypeAliasDecl {
        let start = self.bump().span.start;
        let name = self.expect_identifier().unwrap_or_default();
        let type_parameters = self.parse_type_parameters();
        self.expect_punct(Punctuation::Eq, "'='");
        let ty = self.parse_type();

        TypeAliasDecl {
            name,
            type_parameters,
            ty,
            span: self.span_from(start),
        }
    }

    fn parse_foreign_module(&mut self) -> ForeignModuleDecl {
        let start = self.bump().span.start;
        self.expect_keyword(Keyword::Module, "'module'");
        let name = self.expect_identifier().unwrap_or_default();
        let mut functions = Vec::new();

        self.expect_punct(Punctuation::LBrace, "'{'");
        self.skip_separators();

        while !self.at_punct(Punctuation::RBrace) && !self.at_eof() {
            if self.at_keyword(Keyword::Fn) {
                functions.push(self.parse_foreign_function());
            } else {
                self.error_current(
                    INVALID_ITEM,
                    "Expected a foreign function signature.",
                    "Foreign modules contain only fn declarations without bodies.",
                );
                self.bump();
            }
            self.skip_separators();
        }

        self.expect_punct(Punctuation::RBrace, "'}'");

        ForeignModuleDecl {
            name,
            functions,
            span: self.span_from(start),
        }
    }

    fn parse_foreign_function(&mut self) -> ForeignFunction {
        let start = self.bump().span.start;
        let name = self.expect_identifier().unwrap_or_default();
        let parameters = self.parse_parameters();
        let (return_type, error_set) = self.parse_return_and_errors();

        ForeignFunction {
            name,
            parameters,
            return_type,
            error_set,
            span: self.span_from(start),
        }
    }

    fn parse_script_block(&mut self) -> ScriptBlock {
        let block_start = self
            .expect_punct(Punctuation::LBrace, "'{'")
            .map_or(0, |t| t.span.start);
        self.expect_punct(Punctuation::At, "'@'");
        let _ = self.expect_identifier();
        self.expect_punct(Punctuation::Colon, "':'");

        let lang_token = if self.at(TokenKind::Identifier) {
            Some(self.bump())
        } else {
            self.error_current(
                EXPECTED,
                "Expected script language.",
                "Specify a language such as js, ts, c, cpp, or python.",
            );
            None
        };
        let language = lang_token
            .as_ref()
            .map(|t| t.text.clone())
            .unwrap_or_default();
        let lang_span = lang_token
            .as_ref()
            .map(|t| t.span)
            .unwrap_or_else(|| self.span_from(block_start));
        match language.as_str() {
            "js" | "ts" | "c" | "cpp" | "python" => {}
            "java" => {
                self.error(
                    UNKNOWN_SCRIPT_LANGUAGE,
                    "Script language 'java' is reserved but unsupported until a JVM backend exists.",
                    lang_span,
                    "Supported languages in V1: js, ts, c, cpp, python.",
                );
            }
            _ => {
                self.error(
                    UNKNOWN_SCRIPT_LANGUAGE,
                    format!("Unknown or unsupported script language '{language}'."),
                    lang_span,
                    "Supported languages in V1: js, ts, c, cpp, python.",
                );
            }
        }

        if self.at_keyword(Keyword::As) || self.peek().text == "as" {
            self.bump();
        } else {
            self.error_current(
                EXPECTED,
                "Expected 'as'.",
                "Specify the script namespace using 'as <name>;'.",
            );
        }

        let name = self.expect_identifier().unwrap_or_default();
        self.expect_punct(Punctuation::Semicolon, "';'");
        let header_span = Span::new(block_start, self.last_end());

        let mut mode = String::new();
        let mut exports = Vec::new();
        let mut has_exports = false;

        self.skip_separators();
        while !self.at_eof() && !self.at_punct(Punctuation::RBrace) {
            if self.at_punct(Punctuation::At) {
                if self.peek_at(1).is_some_and(|t| t.text == "impl") {
                    break;
                }
                if self.peek_at(1).is_some_and(|t| t.text == "mode") {
                    self.bump();
                    self.bump();
                    self.expect_punct(Punctuation::LParen, "'('");
                    mode = self.expect_identifier().unwrap_or_default();
                    self.expect_punct(Punctuation::RParen, "')'");
                    self.skip_separators();
                    continue;
                }
                if self.peek_at(1).is_some_and(|t| t.text == "exports") {
                    self.bump();
                    self.bump();
                    has_exports = true;
                    self.expect_punct(Punctuation::LBrace, "'{'");
                    self.skip_separators();
                    let mut seen_exports = std::collections::BTreeSet::new();
                    while !self.at_punct(Punctuation::RBrace) && !self.at_eof() {
                        let before = self.index;
                        if self.at_keyword(Keyword::Fn) {
                            let fn_token = self.bump();
                            let fn_name = self.expect_identifier().unwrap_or_default();
                            if self.at_punct(Punctuation::Lt) {
                                self.error_current(
                                    INVALID_SCRIPT_SIGNATURE,
                                    "Script exports cannot be generic.",
                                    "Remove generic type parameters from the export signature.",
                                );
                                while !self.at_punct(Punctuation::Gt) && !self.at_eof() {
                                    self.bump();
                                }
                                if self.at_punct(Punctuation::Gt) {
                                    self.bump();
                                }
                            }
                            let parameters = self.parse_parameters();
                            let (return_type, error_set) = self.parse_return_and_errors();
                            let exp_span = Span::new(fn_token.span.start, self.last_end());
                            if !seen_exports.insert(fn_name.clone()) {
                                self.error(
                                    INVALID_SCRIPT_EXPORTS,
                                    format!("Duplicate export '{fn_name}' in script block."),
                                    exp_span,
                                    "Remove the duplicate export signature.",
                                );
                            }
                            exports.push(ScriptExport {
                                name: fn_name,
                                parameters,
                                return_type,
                                error_set,
                                span: exp_span,
                            });
                        } else {
                            self.error_current(
                                EXPECTED,
                                "Expected 'fn' export signature in script @exports block.",
                                "Use 'fn <name>(<params>) -> <type>'.",
                            );
                        }
                        self.finish_boundary(true);
                        if self.index == before {
                            self.bump();
                        }
                    }
                    self.expect_punct(Punctuation::RBrace, "'}'");
                    self.skip_separators();
                    continue;
                }
            }
            let tok = self.bump();
            self.error(
                MALFORMED_SCRIPT_BLOCK,
                format!("Unexpected token in script header: '{}'.", tok.text),
                tok.span,
                "Script header must declare @mode(...) and @exports { ... } before @impl.",
            );
        }

        if mode.is_empty() {
            self.error(
                MALFORMED_SCRIPT_BLOCK,
                "Malformed script block: missing @mode(...) declaration.",
                header_span,
                "Declare the minimum mode required by the foreign script, e.g. @mode(core).",
            );
            mode = "core".to_string();
        }

        if !has_exports {
            self.error(
                INVALID_SCRIPT_EXPORTS,
                "Missing @exports declaration in script block.",
                header_span,
                "Declare at least one exported function signature in @exports { ... }.",
            );
        } else if exports.is_empty() {
            self.error(
                INVALID_SCRIPT_EXPORTS,
                "Script block @exports declaration must contain at least one export signature.",
                header_span,
                "Add at least one function signature, e.g. fn add(a: i32, b: i32) -> i32.",
            );
        }

        if self.at_punct(Punctuation::At) && self.peek_at(1).is_some_and(|t| t.text == "impl") {
            self.bump();
            self.bump();
        } else {
            self.error_current(
                MALFORMED_SCRIPT_BLOCK,
                "Malformed script block: missing '@impl' marker.",
                "Mark the start of foreign implementation with '@impl'.",
            );
        }

        let (raw_source, body_span) = if self.at(TokenKind::RawSource) {
            let tok = self.bump();
            (tok.text, tok.span)
        } else {
            (String::new(), Span::new(self.last_end(), self.last_end()))
        };

        if self.at_punct(Punctuation::At) && self.peek_at(1).is_some_and(|t| t.text == "end") {
            self.bump();
            self.bump();
        } else {
            self.error_current(
                MALFORMED_SCRIPT_BLOCK,
                "Malformed script block: missing '@end' sentinel.",
                "End foreign raw source with an '@end' line.",
            );
        }

        self.skip_separators();
        if self.at_punct(Punctuation::RBrace) {
            self.bump();
        } else {
            self.error_current(
                MALFORMED_SCRIPT_BLOCK,
                "Malformed script block: missing closing '}'.",
                "Close the script block with '}'.",
            );
        }

        let total_span = Span::new(block_start, self.last_end());

        ScriptBlock {
            language,
            name,
            mode,
            exports,
            raw_source,
            span: total_span,
            header_span,
            body_span,
        }
    }

    fn parse_scoped_items(&mut self) -> ScopedItems {
        self.depth += 1;
        if self.depth > MAX_NESTING_DEPTH {
            self.nesting_limit_error("Scope nesting exceeds the supported depth.");
            let span = self.peek().span;
            self.depth -= 1;
            return ScopedItems {
                annotation: ScopeAnnotation {
                    kind: ScopeKind::Mode,
                    value: String::new(),
                    span,
                },
                items: Vec::new(),
                span,
            };
        }
        let scoped = self.parse_scoped_items_inner();
        self.depth -= 1;
        scoped
    }

    fn parse_scoped_items_inner(&mut self) -> ScopedItems {
        let start = self.bump().span.start;
        let annotation = self.parse_scope_annotation_body();
        let mut items = Vec::new();

        self.skip_separators();
        while !self.at_punct(Punctuation::RBrace) && !self.at_eof() {
            let before = self.index;
            let attributes = self.parse_attributes();
            if let Some(item) = self.parse_item_with_attributes(attributes) {
                items.push(item);
            }
            self.finish_boundary(true);
            if self.index == before {
                self.bump();
            }
        }

        self.expect_punct(Punctuation::RBrace, "'}'");

        ScopedItems {
            annotation,
            items,
            span: self.span_from(start),
        }
    }

    fn parse_scoped_statements(&mut self) -> ScopedStatements {
        self.depth += 1;
        if self.depth > MAX_NESTING_DEPTH {
            self.nesting_limit_error("Scope nesting exceeds the supported depth.");
            let span = self.peek().span;
            self.depth -= 1;
            return ScopedStatements {
                annotation: ScopeAnnotation {
                    kind: ScopeKind::Mode,
                    value: String::new(),
                    span,
                },
                statements: Vec::new(),
                span,
            };
        }
        let scoped = self.parse_scoped_statements_inner();
        self.depth -= 1;
        scoped
    }

    fn parse_scoped_statements_inner(&mut self) -> ScopedStatements {
        let start = self.bump().span.start;
        let annotation = self.parse_scope_annotation_body();
        let mut statements = Vec::new();

        self.skip_separators();
        while !self.at_punct(Punctuation::RBrace) && !self.at_eof() {
            let before = self.index;
            if self.looks_like_scope_annotation() {
                let scoped = self.parse_scoped_statements();
                statements.push(Stmt {
                    kind: StmtKind::Scoped(scoped.clone()),
                    span: scoped.span,
                });
            } else if let Some(statement) = self.parse_statement() {
                statements.push(statement);
            }
            self.finish_boundary(true);
            if self.index == before {
                self.bump();
            }
        }

        self.expect_punct(Punctuation::RBrace, "'}'");

        ScopedStatements {
            annotation,
            statements,
            span: self.span_from(start),
        }
    }

    fn parse_canonical_scoped_items(&mut self) -> ScopedItems {
        self.depth += 1;
        if self.depth > MAX_NESTING_DEPTH {
            self.nesting_limit_error("Scope nesting exceeds the supported depth.");
            let span = self.peek().span;
            self.depth -= 1;
            return ScopedItems {
                annotation: ScopeAnnotation {
                    kind: ScopeKind::Mode,
                    value: String::new(),
                    span,
                },
                items: Vec::new(),
                span,
            };
        }
        let start = self.peek().span.start;
        let annotation = self.parse_canonical_scope_annotation();
        self.expect_punct(Punctuation::LBrace, "'{'");
        let mut items = Vec::new();

        self.skip_separators();
        while !self.at_punct(Punctuation::RBrace) && !self.at_eof() {
            let before = self.index;
            let attributes = self.parse_attributes();
            if let Some(item) = self.parse_item_with_attributes(attributes) {
                items.push(item);
            }
            self.finish_boundary(true);
            if self.index == before {
                self.bump();
            }
        }

        self.expect_punct(Punctuation::RBrace, "'}'");
        self.depth -= 1;

        ScopedItems {
            annotation,
            items,
            span: self.span_from(start),
        }
    }

    fn parse_canonical_scoped_statements(&mut self) -> ScopedStatements {
        self.depth += 1;
        if self.depth > MAX_NESTING_DEPTH {
            self.nesting_limit_error("Scope nesting exceeds the supported depth.");
            let span = self.peek().span;
            self.depth -= 1;
            return ScopedStatements {
                annotation: ScopeAnnotation {
                    kind: ScopeKind::Mode,
                    value: String::new(),
                    span,
                },
                statements: Vec::new(),
                span,
            };
        }
        let start = self.peek().span.start;
        let annotation = self.parse_canonical_scope_annotation();
        self.expect_punct(Punctuation::LBrace, "'{'");
        let mut statements = Vec::new();

        self.skip_separators();
        while !self.at_punct(Punctuation::RBrace) && !self.at_eof() {
            let before = self.index;
            if let Some(statement) = self.parse_statement() {
                statements.push(statement);
            }
            self.finish_boundary(true);
            if self.index == before {
                self.bump();
            }
        }

        self.expect_punct(Punctuation::RBrace, "'}'");
        self.depth -= 1;

        ScopedStatements {
            annotation,
            statements,
            span: self.span_from(start),
        }
    }

    fn parse_canonical_scope_annotation(&mut self) -> ScopeAnnotation {
        let start = self
            .expect_punct(Punctuation::At, "'@'")
            .map_or_else(|| self.peek().span.start, |token| token.span.start);
        let name = self.expect_identifier().unwrap_or_default();
        self.expect_punct(Punctuation::LParen, "'('");
        self.skip_newlines();
        let value = self.expect_identifier().unwrap_or_default();
        self.skip_newlines();
        self.expect_punct(Punctuation::RParen, "')'");
        let kind = match name.as_str() {
            "mode" => ScopeKind::Mode,
            "memory" => ScopeKind::Memory,
            _ => {
                self.error_current(
                    INVALID,
                    format!("Unknown scope annotation '@{name}'."),
                    "Use @mode(...) or @memory(...).",
                );
                ScopeKind::Mode
            }
        };

        ScopeAnnotation {
            kind,
            value,
            span: self.span_from(start),
        }
    }

    fn parse_scope_annotation_body(&mut self) -> ScopeAnnotation {
        let start = self
            .expect_punct(Punctuation::At, "'@'")
            .map_or_else(|| self.peek().span.start, |token| token.span.start);
        let name = self.expect_identifier().unwrap_or_default();
        self.expect_punct(Punctuation::Colon, "':'");
        let value = self.expect_identifier().unwrap_or_default();
        self.expect_punct(Punctuation::Semicolon, "';'");
        let kind = match name.as_str() {
            "mode" => ScopeKind::Mode,
            "memory" => ScopeKind::Memory,
            _ => {
                self.error_current(
                    INVALID,
                    format!("Unknown scope annotation '@{name}'."),
                    "Use @mode:... or @memory:....",
                );
                ScopeKind::Mode
            }
        };

        ScopeAnnotation {
            kind,
            value,
            span: self.span_from(start),
        }
    }

    fn parse_block(&mut self) -> Block {
        self.depth += 1;
        if self.depth > MAX_NESTING_DEPTH {
            self.nesting_limit_error("Block nesting exceeds the supported depth.");

            self.eat_punct(Punctuation::LBrace);
            let span = self.peek().span;
            self.depth -= 1;
            return Block {
                statements: Vec::new(),
                span,
            };
        }
        let block = self.parse_block_inner();
        self.depth -= 1;
        block
    }

    fn parse_block_inner(&mut self) -> Block {
        let start = self
            .expect_punct(Punctuation::LBrace, "'{'")
            .map_or_else(|| self.peek().span.start, |token| token.span.start);
        let mut statements = Vec::new();

        self.skip_separators();
        self.parse_block_statements(&mut statements);

        self.expect_punct(Punctuation::RBrace, "'}'");

        Block {
            statements,
            span: self.span_from(start),
        }
    }

    fn parse_block_statements(&mut self, statements: &mut Vec<Stmt>) {
        while !self.at_punct(Punctuation::RBrace) && !self.at_eof() {
            let before = self.index;
            if self.looks_like_scope_annotation() {
                let scoped = self.parse_scoped_statements();
                statements.push(Stmt {
                    kind: StmtKind::Scoped(scoped.clone()),
                    span: scoped.span,
                });
            } else if let Some(statement) = self.parse_statement() {
                statements.push(statement);
            }
            self.finish_boundary(true);
            if self.index == before {
                self.bump();
            }
        }
    }

    fn parse_statement(&mut self) -> Option<Stmt> {
        let start = self.peek().span.start;

        if self.at_canonical_scope() {
            let scoped = self.parse_canonical_scoped_statements();
            return Some(Stmt {
                kind: StmtKind::Scoped(scoped.clone()),
                span: scoped.span,
            });
        }

        if self.at_keyword(Keyword::Let) {
            return self.parse_let_statement();
        }

        if self.eat_keyword(Keyword::Return) {
            let value = if self.at(TokenKind::Newline)
                || self.at_punct(Punctuation::Semicolon)
                || self.at_punct(Punctuation::RBrace)
                || self.at_eof()
            {
                None
            } else {
                if self.edition == Edition::V0_2 && self.at_keyword(Keyword::Fail) {
                    self.error(
                        INVALID_CONTROL_FLOW,
                        "'return fail' is redundant in edition 0.2.",
                        self.peek().span,
                        "Remove `return`; `fail` already diverges.",
                    );
                }
                Some(self.parse_expression())
            };
            return Some(Stmt {
                kind: StmtKind::Return { value },
                span: self.span_from(start),
            });
        }

        if self.eat_keyword(Keyword::Break) {
            return Some(Stmt {
                kind: StmtKind::Break,
                span: self.span_from(start),
            });
        }

        if self.eat_keyword(Keyword::Continue) {
            return Some(Stmt {
                kind: StmtKind::Continue,
                span: self.span_from(start),
            });
        }

        if self.eat_keyword(Keyword::Defer) {
            return self.parse_statement().map(|statement| Stmt {
                kind: StmtKind::Defer(Box::new(statement)),
                span: self.span_from(start),
            });
        }

        if self.eat_keyword(Keyword::While) {
            if self.edition == Edition::V0_2 && self.at_keyword(Keyword::Let) {
                self.bump();
                let pattern = self.parse_pattern();
                self.expect_punct(Punctuation::Eq, "'='");
                let value = self.parse_expression();
                let body = self.parse_block();
                return Some(self.desugar_while_let(pattern, value, body, start));
            }
            let condition = self.parse_expression();
            let body = self.parse_block();
            return Some(Stmt {
                kind: StmtKind::While { condition, body },
                span: self.span_from(start),
            });
        }

        if self.eat_keyword(Keyword::For) {
            let binding = self.expect_identifier().unwrap_or_default();
            self.expect_keyword(Keyword::In, "'in'");
            let iterable = self.parse_expression();
            let body = self.parse_block();
            return Some(Stmt {
                kind: StmtKind::For {
                    binding,
                    iterable,
                    body,
                },
                span: self.span_from(start),
            });
        }

        if self.at(TokenKind::Newline)
            || self.at_punct(Punctuation::Semicolon)
            || self.at_punct(Punctuation::RBrace)
            || self.at_eof()
        {
            return None;
        }

        let expression = self.parse_expression();
        Some(Stmt {
            kind: StmtKind::Expr(expression),
            span: self.span_from(start),
        })
    }

    fn parse_let_statement(&mut self) -> Option<Stmt> {
        let start = self.bump().span.start;
        let mutable = self.eat_keyword(Keyword::Mut);
        let name = self.expect_identifier().unwrap_or_default();
        let ty = if self.eat_punct(Punctuation::Colon) {
            Some(self.parse_type())
        } else {
            None
        };

        if !self.eat_punct(Punctuation::Eq) {
            self.error_current(
                EXPECTED,
                "Expected '=' in let binding.",
                "Use 'let name = value' or 'let name: Type = value'.",
            );
            return None;
        }

        let value = self.parse_expression();

        Some(Stmt {
            kind: StmtKind::Let {
                mutable,
                name,
                ty,
                value,
            },
            span: self.span_from(start),
        })
    }

    fn parse_expression(&mut self) -> Expr {
        self.depth += 1;
        self.lambda_depth += 1;
        if self.depth > MAX_NESTING_DEPTH {
            self.lambda_depth -= 1;
            return self.nesting_limit_expression();
        }
        let depth = self.lambda_depth;
        let expression = self.parse_assignment();
        self.depth -= 1;
        self.lambda_depth -= 1;
        if depth == 1 && self.pending_lambda.is_some() {
            let lambda = self.pending_lambda.take().expect("pending lambda");
            self.error(
                EXPECTED,
                "A lambda must be invoked immediately.",
                lambda.span,
                "Write (fn(x) => body)(argument).",
            );
            return Expr {
                kind: ExprKind::Literal(Literal::Unit),
                span: lambda.span,
            };
        }
        expression
    }

    fn nesting_limit_expression(&mut self) -> Expr {
        let span = self.peek().span;
        self.nesting_limit_error("Expression nesting exceeds the supported depth.");
        if self.can_consume_for_progress() {
            self.bump();
        }
        self.depth -= 1;
        Expr {
            kind: ExprKind::Literal(Literal::Unit),
            span,
        }
    }

    fn can_consume_for_progress(&self) -> bool {
        !self.at_eof()
            && !self.at(TokenKind::Newline)
            && !self.at_punct(Punctuation::Semicolon)
            && !self.at_punct(Punctuation::RBrace)
            && !self.at_punct(Punctuation::RParen)
            && !self.at_punct(Punctuation::RBracket)
            && !self.at_punct(Punctuation::Comma)
    }

    fn nesting_limit_error(&mut self, message: &str) {
        if self.depth_reported {
            return;
        }
        self.depth_reported = true;

        self.index = self.tokens.len().saturating_sub(1);
        self.nesting_drained = true;
        let span = self.peek().span;
        self.error(
            INVALID,
            message,
            span,
            "Simplify the expression; the parser supports at most 64 levels of nesting.",
        );
    }

    fn parse_assignment(&mut self) -> Expr {
        let left = self.parse_range();

        if self.eat_punct(Punctuation::Eq) {
            self.skip_continuation_newlines();
            let value = self.parse_assignment();
            let span = Span::new(left.span.start, value.span.end);
            return Expr {
                kind: ExprKind::Assign {
                    target: Box::new(left),
                    value: Box::new(value),
                },
                span,
            };
        }

        left
    }

    fn parse_range(&mut self) -> Expr {
        let start = self.parse_logical_or();
        if self.edition != Edition::V0_2 {
            return start;
        }
        let inclusive = if self.eat_punct(Punctuation::DotDotEq) {
            true
        } else if self.eat_punct(Punctuation::DotDot) {
            false
        } else {
            return start;
        };
        self.skip_continuation_newlines();
        let end = self.parse_logical_or();
        let span = Span::new(start.span.start, end.span.end);
        Expr {
            kind: ExprKind::Range {
                start: Box::new(start),
                end: Box::new(end),
                inclusive,
            },
            span,
        }
    }

    fn parse_logical_or(&mut self) -> Expr {
        let mut expression = self.parse_logical_and();

        while self.eat_punct(Punctuation::PipePipe) {
            self.skip_continuation_newlines();
            let right = self.parse_logical_and();
            let span = Span::new(expression.span.start, right.span.end);
            expression = Expr {
                kind: ExprKind::Binary {
                    operator: BinaryOp::Or,
                    left: Box::new(expression),
                    right: Box::new(right),
                },
                span,
            };
        }

        expression
    }

    fn parse_logical_and(&mut self) -> Expr {
        let mut expression = self.parse_equality();

        while self.eat_punct(Punctuation::AmpAmp) {
            self.skip_continuation_newlines();
            let right = self.parse_equality();
            let span = Span::new(expression.span.start, right.span.end);
            expression = Expr {
                kind: ExprKind::Binary {
                    operator: BinaryOp::And,
                    left: Box::new(expression),
                    right: Box::new(right),
                },
                span,
            };
        }

        expression
    }

    fn parse_bit_or(&mut self) -> Expr {
        let mut expression = self.parse_bit_xor();

        while self.edition == Edition::V0_2 && self.eat_punct(Punctuation::Pipe) {
            self.skip_continuation_newlines();
            let right = self.parse_bit_xor();
            let span = Span::new(expression.span.start, right.span.end);
            expression = Expr {
                kind: ExprKind::Binary {
                    operator: BinaryOp::BitOr,
                    left: Box::new(expression),
                    right: Box::new(right),
                },
                span,
            };
        }

        expression
    }

    fn parse_bit_xor(&mut self) -> Expr {
        let mut expression = self.parse_bit_and();

        while self.edition == Edition::V0_2 && self.eat_punct(Punctuation::Caret) {
            self.skip_continuation_newlines();
            let right = self.parse_bit_and();
            let span = Span::new(expression.span.start, right.span.end);
            expression = Expr {
                kind: ExprKind::Binary {
                    operator: BinaryOp::BitXor,
                    left: Box::new(expression),
                    right: Box::new(right),
                },
                span,
            };
        }

        expression
    }

    fn parse_bit_and(&mut self) -> Expr {
        let mut expression = self.parse_shift();

        while self.edition == Edition::V0_2 && self.eat_punct(Punctuation::Amp) {
            self.skip_continuation_newlines();
            let right = self.parse_shift();
            let span = Span::new(expression.span.start, right.span.end);
            expression = Expr {
                kind: ExprKind::Binary {
                    operator: BinaryOp::BitAnd,
                    left: Box::new(expression),
                    right: Box::new(right),
                },
                span,
            };
        }

        expression
    }

    fn parse_shift(&mut self) -> Expr {
        let mut expression = self.parse_additive();

        loop {
            let operator = if self.edition != Edition::V0_2 {
                None
            } else {
                let current = self.peek();
                let next = self.peek_at(1);
                let adjacent = next.is_some_and(|token| token.span.start == current.span.end);
                if adjacent
                    && current.kind == TokenKind::Punct(Punctuation::Lt)
                    && next.is_some_and(|token| token.kind == TokenKind::Punct(Punctuation::Lt))
                {
                    Some(BinaryOp::Shl)
                } else if adjacent
                    && current.kind == TokenKind::Punct(Punctuation::Gt)
                    && next.is_some_and(|token| token.kind == TokenKind::Punct(Punctuation::Gt))
                {
                    Some(BinaryOp::Shr)
                } else {
                    None
                }
            };
            let Some(operator) = operator else {
                break;
            };
            self.bump();
            self.bump();
            self.skip_continuation_newlines();
            let right = self.parse_additive();
            let span = Span::new(expression.span.start, right.span.end);
            expression = Expr {
                kind: ExprKind::Binary {
                    operator,
                    left: Box::new(expression),
                    right: Box::new(right),
                },
                span,
            };
        }

        expression
    }

    fn parse_equality(&mut self) -> Expr {
        let mut expression = self.parse_comparison();

        loop {
            let operator = if self.eat_punct(Punctuation::EqEq) {
                Some(BinaryOp::Equal)
            } else if self.eat_punct(Punctuation::BangEq) {
                Some(BinaryOp::NotEqual)
            } else {
                None
            };

            let Some(operator) = operator else {
                break;
            };
            self.skip_continuation_newlines();

            let right = self.parse_comparison();
            let span = Span::new(expression.span.start, right.span.end);
            expression = Expr {
                kind: ExprKind::Binary {
                    operator,
                    left: Box::new(expression),
                    right: Box::new(right),
                },
                span,
            };
        }

        expression
    }

    fn parse_comparison(&mut self) -> Expr {
        let mut expression = self.parse_bit_or();

        loop {
            let operator = if self.eat_punct(Punctuation::Lt) {
                Some(BinaryOp::Less)
            } else if self.eat_punct(Punctuation::LtEq) {
                Some(BinaryOp::LessEqual)
            } else if self.eat_punct(Punctuation::Gt) {
                Some(BinaryOp::Greater)
            } else if self.eat_punct(Punctuation::GtEq) {
                Some(BinaryOp::GreaterEqual)
            } else {
                None
            };

            let Some(operator) = operator else {
                break;
            };
            self.skip_continuation_newlines();

            let right = self.parse_bit_or();
            let span = Span::new(expression.span.start, right.span.end);
            expression = Expr {
                kind: ExprKind::Binary {
                    operator,
                    left: Box::new(expression),
                    right: Box::new(right),
                },
                span,
            };
        }

        expression
    }

    fn parse_additive(&mut self) -> Expr {
        let mut expression = self.parse_multiplicative();

        loop {
            let operator = if self.eat_punct(Punctuation::Plus) {
                Some(BinaryOp::Add)
            } else if self.eat_punct(Punctuation::Minus) {
                Some(BinaryOp::Subtract)
            } else if self.eat_punct(Punctuation::PlusWrap) {
                Some(BinaryOp::AddWrap)
            } else if self.eat_punct(Punctuation::MinusWrap) {
                Some(BinaryOp::SubtractWrap)
            } else {
                None
            };

            let Some(operator) = operator else {
                break;
            };
            self.skip_continuation_newlines();

            let right = self.parse_multiplicative();
            let span = Span::new(expression.span.start, right.span.end);
            expression = Expr {
                kind: ExprKind::Binary {
                    operator,
                    left: Box::new(expression),
                    right: Box::new(right),
                },
                span,
            };
        }

        expression
    }

    fn parse_multiplicative(&mut self) -> Expr {
        let mut expression = self.parse_unary();

        loop {
            let operator = if self.eat_punct(Punctuation::Star) {
                Some(BinaryOp::Multiply)
            } else if self.eat_punct(Punctuation::Slash) {
                Some(BinaryOp::Divide)
            } else if self.eat_punct(Punctuation::Percent) {
                Some(BinaryOp::Remainder)
            } else if self.eat_punct(Punctuation::StarWrap) {
                Some(BinaryOp::MultiplyWrap)
            } else if self.eat_punct(Punctuation::SlashWrap) {
                Some(BinaryOp::DivideWrap)
            } else {
                None
            };

            let Some(operator) = operator else {
                break;
            };
            self.skip_continuation_newlines();

            let right = self.parse_unary();
            let span = Span::new(expression.span.start, right.span.end);
            expression = Expr {
                kind: ExprKind::Binary {
                    operator,
                    left: Box::new(expression),
                    right: Box::new(right),
                },
                span,
            };
        }

        expression
    }

    fn parse_unary(&mut self) -> Expr {
        let start = self.peek().span.start;

        let operator = if self.eat_punct(Punctuation::Minus) {
            Some(UnaryOp::Negate)
        } else if self.eat_keyword(Keyword::Not) {
            Some(UnaryOp::Not)
        } else {
            None
        };

        if let Some(operator) = operator {
            let operand = self.parse_unary();
            return Expr {
                kind: ExprKind::Unary {
                    operator,
                    operand: Box::new(operand),
                },
                span: self.span_from(start),
            };
        }

        self.parse_postfix()
    }

    fn parse_postfix(&mut self) -> Expr {
        let mut expression = self.parse_primary();

        if let Some(lambda) = self.pending_lambda.take() {
            if self.eat_punct(Punctuation::LParen) {
                let arguments = self.parse_expression_list_after_open();
                let span = Span::new(lambda.span.start, self.last_end());
                expression = self.lower_lambda_call(lambda, arguments, span);
            } else {
                self.pending_lambda = Some(lambda);
            }
        }

        loop {
            if self.edition == Edition::V0_2
                && self.at(TokenKind::Newline)
                && self.next_non_newline_is_punct(Punctuation::Dot)
            {
                self.skip_newlines();
            }

            if self.eat_punct(Punctuation::Dot) {
                let member = if self.at(TokenKind::Identifier) || self.at(TokenKind::Integer) {
                    self.bump().text
                } else {
                    self.error_current(
                        EXPECTED,
                        "Expected a field name or tuple index after '.'.",
                        "Use .name or .0.",
                    );
                    String::new()
                };

                let span = Span::new(expression.span.start, self.last_end());
                expression = Expr {
                    kind: ExprKind::Member {
                        target: Box::new(expression),
                        member,
                    },
                    span,
                };
                continue;
            }

            if self.eat_punct(Punctuation::LParen) {
                let arguments = self.parse_expression_list_after_open();
                let span = Span::new(expression.span.start, self.last_end());
                expression = Expr {
                    kind: ExprKind::Call {
                        callee: Box::new(expression),
                        arguments,
                    },
                    span,
                };
                continue;
            }

            if self.eat_punct(Punctuation::LBracket) {
                let index = self.parse_expression();
                self.expect_punct(Punctuation::RBracket, "']'");
                let span = Span::new(expression.span.start, self.last_end());
                if self.edition == Edition::V0_2 && matches!(index.kind, ExprKind::Range { .. }) {
                    let index_span = index.span;
                    let ExprKind::Range {
                        start,
                        end,
                        inclusive,
                    } = index.kind
                    else {
                        unreachable!("range was checked above");
                    };

                    let callee = Expr {
                        kind: ExprKind::Member {
                            target: Box::new(expression),
                            member: "slice".to_owned(),
                        },
                        span: index_span,
                    };
                    let end = if inclusive {
                        let end_span = end.span;
                        Expr {
                            kind: ExprKind::Binary {
                                operator: BinaryOp::Add,
                                left: end,
                                right: Box::new(Expr {
                                    kind: ExprKind::Literal(Literal::Integer("1".to_owned())),
                                    span: Span::new(end_span.end, end_span.end),
                                }),
                            },
                            span: end_span,
                        }
                    } else {
                        *end
                    };
                    expression = Expr {
                        kind: ExprKind::Call {
                            callee: Box::new(callee),
                            arguments: vec![*start, end],
                        },
                        span,
                    };
                } else {
                    expression = Expr {
                        kind: ExprKind::Index {
                            target: Box::new(expression),
                            index: Box::new(index),
                        },
                        span,
                    };
                }
                continue;
            }

            if self.eat_punct(Punctuation::Bang) {
                let span = Span::new(expression.span.start, self.last_end());
                if self.edition == Edition::V0_2 {
                    self.error(
                        DEPRECATED_SYNTAX,
                        "Postfix `!` is not allowed in edition 0.2.",
                        span,
                        "Use `?` to propagate a failure.",
                    );
                }
                expression = Expr {
                    kind: ExprKind::Propagate(Box::new(expression)),
                    span,
                };
                continue;
            }

            if self.edition == Edition::V0_2 && self.eat_punct(Punctuation::Question) {
                let span = Span::new(expression.span.start, self.last_end());
                expression = Expr {
                    kind: ExprKind::Propagate(Box::new(expression)),
                    span,
                };
                continue;
            }

            if self.edition == Edition::V0_2 && self.at_punct(Punctuation::DoubleColon) {
                let start = expression.span.start;
                let turbofish_span = self.peek().span;
                if matches!(&expression.kind, ExprKind::TypeArguments { .. }) {
                    self.error(
                        MALFORMED_TURBOFISH,
                        "Only one turbofish is allowed in a postfix chain.",
                        turbofish_span,
                        "Remove the second turbofish.",
                    );
                    self.bump();
                    let _ = self.parse_type_arguments();
                    continue;
                }

                self.bump();
                let arguments = self.parse_type_arguments();
                let span = Span::new(start, self.last_end());
                let target_is_path = matches!(
                    &expression.kind,
                    ExprKind::Identifier(_) | ExprKind::Member { .. }
                );
                let followed_by_call = self.at_punct(Punctuation::LParen);
                let followed_by_struct_init =
                    self.at_punct(Punctuation::LBrace) && self.looks_like_struct_init();
                if !target_is_path {
                    self.error(
                        MALFORMED_TURBOFISH,
                        "A turbofish must follow a path.",
                        span,
                        "Use path::<...>(...) or path::<...> { ... }.",
                    );
                } else if !followed_by_call
                    && !followed_by_struct_init
                    && matches!(&expression.kind, ExprKind::Member { .. })
                {
                    self.error(
                        BARE_TYPE_ARGUMENTS,
                        "A qualified value path cannot carry expression-position type arguments.",
                        span,
                        "Use the bare variant name, such as None::<i32>.",
                    );
                }
                expression = Expr {
                    kind: ExprKind::TypeArguments {
                        target: Box::new(expression),
                        arguments,
                    },
                    span,
                };
                continue;
            }

            if self.edition == Edition::V0_2 && self.at_punct(Punctuation::Lt) {
                let lt_span = self.peek().span;
                if let Some(arguments) = self.try_type_arguments_for_call() {
                    self.error(
                        BARE_TYPE_ARGUMENTS,
                        "Expression-position type arguments require `::<...>` in edition 0.2.",
                        lt_span,
                        "Write path::<T>(value) instead of path<T>(value).",
                    );
                    let span = Span::new(expression.span.start, self.last_end());
                    expression = Expr {
                        kind: ExprKind::TypeArguments {
                            target: Box::new(expression),
                            arguments,
                        },
                        span,
                    };
                    continue;
                }
            }

            if self.edition == Edition::V0_1
                && self.at_punct(Punctuation::Lt)
                && let Some(arguments) = self.try_type_arguments_for_call()
            {
                let span = Span::new(expression.span.start, self.last_end());
                expression = Expr {
                    kind: ExprKind::TypeArguments {
                        target: Box::new(expression),
                        arguments,
                    },
                    span,
                };
                continue;
            }

            if self.at_punct(Punctuation::LBrace)
                && self.looks_like_struct_init()
                && let Some(path) = expression_to_type_path(&expression)
            {
                let fields = self.parse_struct_init_fields();
                let span = Span::new(expression.span.start, self.last_end());
                expression = Expr {
                    kind: ExprKind::StructInit { path, fields },
                    span,
                };
                continue;
            }

            break;
        }

        expression
    }

    fn parse_lambda(&mut self) -> Expr {
        let start = self.bump().span.start;
        self.expect_punct(Punctuation::LParen, "'('");
        let mut parameters = Vec::new();
        if !self.at_punct(Punctuation::RParen) {
            loop {
                let token = self.peek().clone();
                let name = if self.at(TokenKind::Identifier) {
                    self.bump().text
                } else {
                    self.error_current(
                        EXPECTED,
                        "Expected a lambda parameter name.",
                        "Write fn(name) => expression.",
                    );
                    String::new()
                };
                let ty = if self.eat_punct(Punctuation::Colon) {
                    Some(self.parse_type())
                } else {
                    None
                };
                parameters.push(LambdaParameter {
                    name,
                    ty,
                    span: token.span,
                });
                if !self.eat_punct(Punctuation::Comma) {
                    break;
                }
                self.skip_newlines();
            }
        }
        self.expect_punct(Punctuation::RParen, "')'");
        self.expect_punct(Punctuation::FatArrow, "'=>'");
        self.skip_continuation_newlines();
        let body = Box::new(self.parse_assignment());
        let span = Span::new(start, body.span.end);
        self.pending_lambda = Some(PendingLambda {
            parameters,
            body,
            span,
        });
        Expr {
            kind: ExprKind::Identifier("__lambda".to_owned()),
            span,
        }
    }

    fn lower_lambda_call(
        &mut self,
        lambda: PendingLambda,
        arguments: Vec<Expr>,
        span: Span,
    ) -> Expr {
        let mut statements = Vec::new();
        let body = lambda.body;
        let body_span = body.span;
        let arity = lambda.parameters.len();
        if arguments.len() != arity {
            self.error(
                EXPECTED,
                format!(
                    "Lambda expects {arity} argument(s), but {} were provided.",
                    arguments.len()
                ),
                span,
                "Match the lambda parameter list.",
            );
        }
        for (index, parameter) in lambda.parameters.into_iter().enumerate() {
            let value = arguments.get(index).cloned().unwrap_or(Expr {
                kind: ExprKind::Literal(Literal::Unit),
                span: parameter.span,
            });
            statements.push(Stmt {
                kind: StmtKind::Let {
                    mutable: false,
                    name: parameter.name,
                    ty: parameter.ty,
                    value,
                },
                span: parameter.span,
            });
        }
        statements.push(Stmt {
            kind: StmtKind::Expr(*body),
            span: body_span,
        });
        Expr {
            kind: ExprKind::Block(Block { statements, span }),
            span,
        }
    }

    fn parse_primary(&mut self) -> Expr {
        let token = self.peek().clone();
        let start = token.span.start;

        match token.kind {
            TokenKind::Integer => {
                self.bump();
                Expr {
                    kind: ExprKind::Literal(Literal::Integer(token.text)),
                    span: token.span,
                }
            }
            TokenKind::Float => {
                self.bump();
                Expr {
                    kind: ExprKind::Literal(Literal::Float(token.text)),
                    span: token.span,
                }
            }
            TokenKind::String => {
                self.bump();
                let value = match decode_string_literal(&token.text) {
                    Ok(value) => value,
                    Err(message) => {
                        self.error(INVALID, message, token.span, "Fix the string escape.");
                        String::new()
                    }
                };
                Expr {
                    kind: ExprKind::Literal(Literal::String(value)),
                    span: token.span,
                }
            }
            TokenKind::ByteString => {
                self.bump();
                let value = match decode_byte_literal(&token.text) {
                    Ok(value) => value,
                    Err(message) => {
                        self.error(INVALID, message, token.span, "Fix the byte escape.");
                        Vec::new()
                    }
                };
                Expr {
                    kind: ExprKind::Literal(Literal::Bytes(value)),
                    span: token.span,
                }
            }
            TokenKind::Character => {
                self.bump();
                let value = match decode_char_literal(&token.text) {
                    Ok(value) => value,
                    Err(message) => {
                        self.error(INVALID, message, token.span, "Fix the character literal.");
                        '\0'
                    }
                };
                Expr {
                    kind: ExprKind::Literal(Literal::Character(value)),
                    span: token.span,
                }
            }
            TokenKind::Keyword(Keyword::True) => {
                self.bump();
                Expr {
                    kind: ExprKind::Literal(Literal::Bool(true)),
                    span: token.span,
                }
            }
            TokenKind::Keyword(Keyword::False) => {
                self.bump();
                Expr {
                    kind: ExprKind::Literal(Literal::Bool(false)),
                    span: token.span,
                }
            }
            TokenKind::Identifier => {
                self.bump();
                Expr {
                    kind: ExprKind::Identifier(token.text),
                    span: token.span,
                }
            }
            TokenKind::Punct(Punctuation::LParen) => self.parse_parenthesized(),
            TokenKind::Punct(Punctuation::LBracket) => self.parse_array_literal(),
            TokenKind::Punct(Punctuation::LBrace) => self.parse_brace_expression(),
            TokenKind::Punct(Punctuation::Bang) => {
                if self.edition == Edition::V0_2 {
                    self.error(
                        DEPRECATED_SYNTAX,
                        "Prefix `!` failure construction is not allowed in edition 0.2.",
                        token.span,
                        "Use `fail Error(...)` or `fail Error { field: value }`.",
                    );
                }
                self.parse_fail_expression()
            }
            TokenKind::Keyword(Keyword::Fail) => self.parse_fail_keyword_expression(),
            TokenKind::Keyword(Keyword::If) => self.parse_if_expression(),
            TokenKind::Keyword(Keyword::Match) => self.parse_match_expression(),
            TokenKind::Keyword(Keyword::Spawn) => self.parse_spawn_expression(),
            TokenKind::Keyword(Keyword::Panic) => {
                self.parse_message_expression(|message| ExprKind::Panic { message })
            }
            TokenKind::Keyword(Keyword::Abort) => {
                self.parse_message_expression(|message| ExprKind::Abort { message })
            }
            TokenKind::Keyword(Keyword::Fn) if self.edition == Edition::V0_2 => self.parse_lambda(),
            _ => {
                let (message, hint) =
                    if self.edition == Edition::V0_2 && is_binary_operator_token(&token.kind) {
                        (
                            "A binary operator cannot start an expression.",
                            "Add a left-hand operand before the operator.",
                        )
                    } else {
                        (
                            "Expected an expression.",
                            "Add a literal, identifier, call, block, if, or match expression.",
                        )
                    };
                self.error(INVALID, message, token.span, hint);
                if !self.at_eof()
                    && !self.at(TokenKind::Newline)
                    && !self.at_punct(Punctuation::Semicolon)
                    && !self.at_punct(Punctuation::RBrace)
                    && !self.at_punct(Punctuation::RParen)
                    && !self.at_punct(Punctuation::RBracket)
                {
                    self.bump();
                }
                Expr {
                    kind: ExprKind::Literal(Literal::Unit),
                    span: Span::new(start, self.last_end()),
                }
            }
        }
    }

    fn parse_parenthesized(&mut self) -> Expr {
        let start = self.bump().span.start;

        if self.eat_punct(Punctuation::RParen) {
            return Expr {
                kind: ExprKind::Literal(Literal::Unit),
                span: self.span_from(start),
            };
        }

        let first = self.parse_expression();
        if matches!(&first.kind, ExprKind::Identifier(name) if name == "__lambda")
            && let Some(lambda) = self.pending_lambda.as_mut()
        {
            lambda.span = Span::new(start, lambda.span.end);
        }
        self.skip_newlines();

        if self.eat_punct(Punctuation::Comma) {
            let mut elements = vec![first];
            loop {
                self.skip_newlines();
                if self.edition == Edition::V0_2 && self.at_punct(Punctuation::RParen) {
                    break;
                }
                elements.push(self.parse_expression());
                self.skip_newlines();
                if self.eat_punct(Punctuation::Comma) {
                    continue;
                }
                break;
            }
            self.skip_newlines();
            self.expect_punct(Punctuation::RParen, "')'");
            return Expr {
                kind: ExprKind::Tuple(elements),
                span: self.span_from(start),
            };
        }

        self.expect_punct(Punctuation::RParen, "')'");
        let mut expression = first;
        expression.span = self.span_from(start);
        expression
    }

    fn parse_array_literal(&mut self) -> Expr {
        let start = self.bump().span.start;
        let mut elements = Vec::new();
        self.skip_newlines();

        if !self.at_punct(Punctuation::RBracket) {
            loop {
                elements.push(self.parse_expression());
                if self.eat_punct(Punctuation::Comma) {
                    self.skip_newlines();
                    if self.at_punct(Punctuation::RBracket) {
                        break;
                    }
                    continue;
                }
                break;
            }
        }

        self.skip_newlines();
        self.expect_punct(Punctuation::RBracket, "']'");
        Expr {
            kind: ExprKind::Array(elements),
            span: self.span_from(start),
        }
    }

    fn parse_brace_expression(&mut self) -> Expr {
        let saved_index = self.index;
        let saved_diagnostics = self.diagnostics.len();
        let start = self.bump().span.start;
        self.skip_newlines();

        if self.at_punct(Punctuation::RBrace) {
            if !self.nesting_drained {
                self.index = saved_index;
            }
            self.diagnostics.truncate(saved_diagnostics);
            return Expr {
                kind: ExprKind::Block(self.parse_block()),
                span: self.span_from(start),
            };
        }

        let first = self.parse_expression();

        if self.eat_punct(Punctuation::Colon) {
            let mut entries = Vec::new();
            let first_value = self.parse_expression();
            entries.push(MapEntry {
                key: first,
                value: first_value,
                span: self.span_from(start),
            });

            loop {
                self.skip_newlines();
                if self.eat_punct(Punctuation::Comma) {
                    self.skip_newlines();
                    if self.at_punct(Punctuation::RBrace) {
                        break;
                    }
                    let key_start = self.peek().span.start;
                    let key = self.parse_expression();
                    self.expect_punct(Punctuation::Colon, "':'");
                    let value = self.parse_expression();
                    entries.push(MapEntry {
                        key,
                        value,
                        span: self.span_from(key_start),
                    });
                    continue;
                }
                break;
            }

            self.expect_punct(Punctuation::RBrace, "'}'");
            return Expr {
                kind: ExprKind::Map(entries),
                span: self.span_from(start),
            };
        }

        if self.diagnostics.len() == saved_diagnostics {
            let statement_start = first.span.start;
            let statement = Stmt {
                kind: StmtKind::Expr(first),
                span: self.span_from(statement_start),
            };
            let mut statements = vec![statement];
            self.finish_boundary(true);
            self.parse_block_statements(&mut statements);
            self.expect_punct(Punctuation::RBrace, "'}'");

            return Expr {
                kind: ExprKind::Block(Block {
                    statements,
                    span: self.span_from(start),
                }),
                span: self.span_from(start),
            };
        }

        if !self.nesting_drained {
            self.index = saved_index;
        }
        self.diagnostics.truncate(saved_diagnostics);
        Expr {
            kind: ExprKind::Block(self.parse_block()),
            span: self.span_from(start),
        }
    }

    fn parse_if_expression(&mut self) -> Expr {
        let start = self.bump().span.start;

        if self.edition == Edition::V0_2 && self.at_keyword(Keyword::Let) {
            self.bump();
            let pattern = self.parse_pattern();
            self.expect_punct(Punctuation::Eq, "'='");
            let value = self.parse_expression();
            let then_branch = self.parse_block();
            let else_branch = if self.eat_keyword(Keyword::Else) {
                if self.at_keyword(Keyword::If) {
                    Some(ElseBranch::If(Box::new(self.parse_if_expression())))
                } else {
                    Some(ElseBranch::Block(self.parse_block()))
                }
            } else {
                None
            };
            return self.desugar_if_let(pattern, value, then_branch, else_branch, start);
        }

        let condition = self.parse_expression();
        if self.edition == Edition::V0_2 && expression_has_unparenthesized_struct_init(&condition) {
            self.error(
                EXPECTED,
                "Wrap a struct initializer in parentheses in a condition.",
                condition.span,
                "Write (User { field: value }) instead of User { field: value }.",
            );
        }
        let then_branch = self.parse_block();
        let else_branch = if self.eat_keyword(Keyword::Else) {
            if self.at_keyword(Keyword::If) {
                Some(ElseBranch::If(Box::new(self.parse_if_expression())))
            } else {
                Some(ElseBranch::Block(self.parse_block()))
            }
        } else {
            None
        };

        Expr {
            kind: ExprKind::If {
                condition: Box::new(condition),
                then_branch,
                else_branch,
            },
            span: self.span_from(start),
        }
    }

    fn desugar_if_let(
        &self,
        pattern: Pattern,
        value: Expr,
        then_branch: Block,
        else_branch: Option<ElseBranch>,
        start: usize,
    ) -> Expr {
        let span = self.span_from(start);
        let else_body = match else_branch {
            Some(ElseBranch::Block(block)) => MatchArmBody::Block(block),
            Some(ElseBranch::If(expression)) => MatchArmBody::Expr(*expression),
            None => MatchArmBody::Expr(Expr {
                kind: ExprKind::Literal(Literal::Unit),
                span,
            }),
        };
        let arms = vec![
            MatchArm {
                pattern,
                body: MatchArmBody::Block(then_branch),
                span,
            },
            MatchArm {
                pattern: Pattern {
                    kind: PatternKind::Wildcard,
                    span,
                },
                body: else_body,
                span,
            },
        ];
        Expr {
            kind: ExprKind::Match {
                value: Box::new(value),
                arms,
            },
            span,
        }
    }

    fn desugar_while_let(&self, pattern: Pattern, value: Expr, body: Block, start: usize) -> Stmt {
        let span = self.span_from(start);
        let match_expression = Expr {
            kind: ExprKind::Match {
                value: Box::new(value),
                arms: vec![
                    MatchArm {
                        pattern,
                        body: MatchArmBody::Block(body),
                        span,
                    },
                    MatchArm {
                        pattern: Pattern {
                            kind: PatternKind::Wildcard,
                            span,
                        },
                        body: MatchArmBody::Block(Block {
                            statements: vec![Stmt {
                                kind: StmtKind::Break,
                                span,
                            }],
                            span,
                        }),
                        span,
                    },
                ],
            },
            span,
        };
        Stmt {
            kind: StmtKind::While {
                condition: Expr {
                    kind: ExprKind::Literal(Literal::Bool(true)),
                    span,
                },
                body: Block {
                    statements: vec![Stmt {
                        kind: StmtKind::Expr(match_expression),
                        span,
                    }],
                    span,
                },
            },
            span,
        }
    }

    fn parse_match_expression(&mut self) -> Expr {
        let start = self.bump().span.start;
        let value = self.parse_expression();
        let mut arms = Vec::new();

        self.expect_punct(Punctuation::LBrace, "'{'");
        self.skip_newlines();

        while !self.at_punct(Punctuation::RBrace) && !self.at_eof() {
            let before = self.index;
            let arm_start = self.peek().span.start;
            let pattern = self.parse_pattern();
            self.expect_punct(Punctuation::FatArrow, "'=>'");
            let body = if self.at_punct(Punctuation::LBrace) {
                MatchArmBody::Block(self.parse_block())
            } else if self.at_keyword(Keyword::Return) {
                let statement = self
                    .parse_statement()
                    .expect("a return keyword always parses a statement");
                let span = statement.span;
                MatchArmBody::Block(Block {
                    statements: vec![statement],
                    span,
                })
            } else {
                MatchArmBody::Expr(self.parse_expression())
            };
            let block_body = matches!(body, MatchArmBody::Block(_));
            arms.push(MatchArm {
                pattern,
                body,
                span: self.span_from(arm_start),
            });

            if self.index == before {
                self.bump();
            }
            if self.eat_punct(Punctuation::Comma) {
                self.skip_newlines();
            } else {
                let next = self.skip_newlines_at(self.index);
                let at_close = self
                    .tokens
                    .get(next)
                    .is_some_and(|token| token.kind == TokenKind::Punct(Punctuation::RBrace));
                if self.edition == Edition::V0_2 && !block_body && !at_close && !self.at_eof() {
                    self.error(
                        BOUNDARY,
                        "Expected ',' after a match arm.",
                        self.span_from(arm_start),
                        "Add a comma between match arms, or use a block body to omit it.",
                    );
                }
                self.skip_newlines();
            }
        }

        self.expect_punct(Punctuation::RBrace, "'}'");

        Expr {
            kind: ExprKind::Match {
                value: Box::new(value),
                arms,
            },
            span: self.span_from(start),
        }
    }

    fn parse_spawn_expression(&mut self) -> Expr {
        let start = self.bump().span.start;
        if self.edition == Edition::V0_2 && self.at_punct(Punctuation::LBrace) {
            let body = self.parse_block();
            return Expr {
                kind: ExprKind::Spawn { body },
                span: self.span_from(start),
            };
        }
        self.expect_punct(Punctuation::PipePipe, "'||'");
        let body = self.parse_block();
        Expr {
            kind: ExprKind::Spawn { body },
            span: self.span_from(start),
        }
    }

    fn parse_fail_expression(&mut self) -> Expr {
        let start = self.bump().span.start;
        self.parse_fail_tail(start, "!Error(args) or !Error { field: value }")
    }

    fn parse_fail_keyword_expression(&mut self) -> Expr {
        let start = self.bump().span.start;
        self.parse_fail_tail(start, "fail Error(args) or fail Error { field: value }")
    }

    fn parse_fail_tail(&mut self, start: usize, example: &str) -> Expr {
        let error = self.parse_type_path();

        let arguments = if self.eat_punct(Punctuation::LParen) {
            FailArguments::Tuple(self.parse_expression_list_after_open())
        } else if self.at_punct(Punctuation::LBrace) {
            FailArguments::Struct(self.parse_struct_init_fields())
        } else {
            self.error_current(
                EXPECTED,
                "Expected '(' or '{' after a failure type.",
                format!("Construct a failure with {example}."),
            );
            FailArguments::Tuple(Vec::new())
        };

        Expr {
            kind: ExprKind::Fail { error, arguments },
            span: self.span_from(start),
        }
    }

    fn parse_message_expression(&mut self, build: fn(Box<Expr>) -> ExprKind) -> Expr {
        let start = self.bump().span.start;
        self.expect_punct(Punctuation::LParen, "'('");
        let message = self.parse_expression();
        self.expect_punct(Punctuation::RParen, "')'");
        Expr {
            kind: build(Box::new(message)),
            span: self.span_from(start),
        }
    }

    fn parse_expression_list_after_open(&mut self) -> Vec<Expr> {
        let mut arguments = Vec::new();
        self.skip_newlines();

        if self.at_punct(Punctuation::RParen) {
            self.bump();
            return arguments;
        }

        loop {
            arguments.push(self.parse_expression());
            if self.eat_punct(Punctuation::Comma) {
                self.skip_newlines();
                if self.at_punct(Punctuation::RParen) {
                    break;
                }
                continue;
            }
            break;
        }

        self.skip_newlines();
        self.expect_punct(Punctuation::RParen, "')'");
        arguments
    }

    fn try_type_arguments_for_call(&mut self) -> Option<Vec<Type>> {
        let saved_index = self.index;
        let saved_diagnostics = self.diagnostics.len();
        let arguments = self.parse_type_arguments();

        let followed_by_call = self.at_punct(Punctuation::LParen);
        let followed_by_struct_init =
            self.at_punct(Punctuation::LBrace) && self.looks_like_struct_init();
        if (followed_by_call || followed_by_struct_init)
            && self.diagnostics.len() == saved_diagnostics
        {
            Some(arguments)
        } else {
            if !self.nesting_drained {
                self.index = saved_index;
            }
            self.diagnostics.truncate(saved_diagnostics);
            None
        }
    }

    fn parse_type_arguments(&mut self) -> Vec<Type> {
        let mut arguments = Vec::new();
        self.expect_punct(Punctuation::Lt, "'<'");

        if !self.at_punct(Punctuation::Gt) {
            loop {
                arguments.push(self.parse_type());
                if self.eat_punct(Punctuation::Comma) {
                    self.skip_newlines();
                    if self.edition == Edition::V0_2 && self.at_punct(Punctuation::Gt) {
                        break;
                    }
                    continue;
                }
                break;
            }
        }

        self.skip_newlines();
        self.expect_punct(Punctuation::Gt, "'>'");
        arguments
    }

    fn parse_struct_init_fields(&mut self) -> Vec<FieldInit> {
        let mut fields = Vec::new();
        self.expect_punct(Punctuation::LBrace, "'{'");
        self.skip_newlines();

        while !self.at_punct(Punctuation::RBrace) && !self.at_eof() {
            let start = self.peek().span.start;
            let name = self.expect_identifier().unwrap_or_default();
            self.expect_punct(Punctuation::Colon, "':'");
            let value = self.parse_expression();
            fields.push(FieldInit {
                name,
                value,
                span: self.span_from(start),
            });

            self.skip_newlines();
            if self.eat_punct(Punctuation::Comma) {
                self.skip_newlines();
                continue;
            }
            break;
        }

        self.skip_newlines();
        self.expect_punct(Punctuation::RBrace, "'}'");
        fields
    }

    fn parse_pattern(&mut self) -> Pattern {
        let token = self.peek().clone();
        let start = token.span.start;

        if token.kind == TokenKind::Punct(Punctuation::Underscore) {
            self.bump();
            return Pattern {
                kind: PatternKind::Wildcard,
                span: token.span,
            };
        }

        if token.kind == TokenKind::Punct(Punctuation::LParen) {
            self.bump();
            let mut patterns = Vec::new();
            self.skip_newlines();

            if !self.at_punct(Punctuation::RParen) {
                loop {
                    patterns.push(self.parse_pattern());
                    if self.eat_punct(Punctuation::Comma) {
                        self.skip_newlines();
                        if self.edition == Edition::V0_2 && self.at_punct(Punctuation::RParen) {
                            break;
                        }
                        continue;
                    }
                    break;
                }
            }

            self.expect_punct(Punctuation::RParen, "')'");
            return Pattern {
                kind: PatternKind::Tuple(patterns),
                span: self.span_from(start),
            };
        }

        if let Some(literal) = self.pattern_literal(&token) {
            self.bump();
            return Pattern {
                kind: PatternKind::Literal(literal),
                span: token.span,
            };
        }

        if token.kind == TokenKind::Identifier {
            self.bump();
            let first = token.text;

            if self.eat_punct(Punctuation::Dot) {
                let variant = self.expect_identifier().unwrap_or_default();
                let arguments = if self.eat_punct(Punctuation::LParen) {
                    Some(self.parse_pattern_list_after_open())
                } else {
                    None
                };
                let path = TypePath {
                    segments: vec![first],
                    arguments: Vec::new(),
                    span: token.span,
                };
                return Pattern {
                    kind: PatternKind::Variant {
                        path,
                        variant,
                        arguments,
                    },
                    span: self.span_from(start),
                };
            }

            if self.eat_punct(Punctuation::LParen) {
                let arguments = self.parse_pattern_list_after_open();
                let path = TypePath {
                    segments: vec![first],
                    arguments: Vec::new(),
                    span: token.span,
                };
                return Pattern {
                    kind: PatternKind::Constructor { path, arguments },
                    span: self.span_from(start),
                };
            }

            return Pattern {
                kind: PatternKind::Identifier(first),
                span: token.span,
            };
        }

        self.error(
            INVALID_PATTERN,
            "Expected a pattern.",
            token.span,
            "Use _, a binding, a literal, an enum variant, or a tuple pattern.",
        );
        if !self.at_eof()
            && !self.at(TokenKind::Newline)
            && !self.at_punct(Punctuation::Semicolon)
            && !self.at_punct(Punctuation::RBrace)
            && !self.at_punct(Punctuation::FatArrow)
        {
            self.bump();
        }

        Pattern {
            kind: PatternKind::Wildcard,
            span: Span::new(start, self.last_end()),
        }
    }

    fn parse_pattern_list_after_open(&mut self) -> Vec<Pattern> {
        let mut patterns = Vec::new();
        self.skip_newlines();

        if self.at_punct(Punctuation::RParen) {
            self.bump();
            return patterns;
        }

        loop {
            patterns.push(self.parse_pattern());
            if self.eat_punct(Punctuation::Comma) {
                self.skip_newlines();
                if self.at_punct(Punctuation::RParen) {
                    break;
                }
                continue;
            }
            break;
        }

        self.skip_newlines();
        self.expect_punct(Punctuation::RParen, "')'");
        patterns
    }

    fn pattern_literal(&self, token: &Token) -> Option<Literal> {
        match token.kind {
            TokenKind::Integer => Some(Literal::Integer(token.text.clone())),
            TokenKind::Float => Some(Literal::Float(token.text.clone())),
            TokenKind::String => decode_string_literal(&token.text).ok().map(Literal::String),
            TokenKind::ByteString => decode_byte_literal(&token.text).ok().map(Literal::Bytes),
            TokenKind::Character => decode_char_literal(&token.text)
                .ok()
                .map(Literal::Character),
            TokenKind::Keyword(Keyword::True) => Some(Literal::Bool(true)),
            TokenKind::Keyword(Keyword::False) => Some(Literal::Bool(false)),
            _ => None,
        }
    }

    fn parse_type(&mut self) -> Type {
        self.depth += 1;
        if self.depth > MAX_NESTING_DEPTH {
            let span = self.peek().span;
            self.nesting_limit_error("Type nesting exceeds the supported depth.");
            if self.can_consume_for_progress() {
                self.bump();
            }
            self.depth -= 1;
            return Type {
                kind: TypeKind::Unit,
                span,
            };
        }
        let ty = self.parse_type_inner();
        self.depth -= 1;
        ty
    }

    fn parse_type_inner(&mut self) -> Type {
        let token = self.peek().clone();
        let start = token.span.start;

        match token.kind {
            TokenKind::Punct(Punctuation::Underscore) if self.edition == Edition::V0_2 => {
                self.bump();
                self.error(
                    INVALID_TYPE_ARGUMENT,
                    "Wildcard type arguments are not supported in edition 0.2.",
                    token.span,
                    "Write an explicit type instead of `_`.",
                );
                Type {
                    kind: TypeKind::Unit,
                    span: token.span,
                }
            }
            TokenKind::Punct(Punctuation::LBracket) => {
                self.bump();
                let element = self.parse_type();

                if self.eat_punct(Punctuation::Semicolon) {
                    let length = self.expect_integer();
                    self.expect_punct(Punctuation::RBracket, "']'");
                    Type {
                        kind: TypeKind::FixedArray(Box::new(element), length),
                        span: self.span_from(start),
                    }
                } else {
                    self.expect_punct(Punctuation::RBracket, "']'");
                    Type {
                        kind: TypeKind::Array(Box::new(element)),
                        span: self.span_from(start),
                    }
                }
            }
            TokenKind::Punct(Punctuation::LBrace) => {
                self.bump();
                let key = self.parse_type();
                self.expect_punct(Punctuation::Colon, "':'");
                let value = self.parse_type();
                self.expect_punct(Punctuation::RBrace, "'}'");
                Type {
                    kind: TypeKind::Map(Box::new(key), Box::new(value)),
                    span: self.span_from(start),
                }
            }
            TokenKind::Punct(Punctuation::LParen) => {
                self.bump();

                if self.eat_punct(Punctuation::RParen) {
                    return Type {
                        kind: TypeKind::Unit,
                        span: self.span_from(start),
                    };
                }

                let mut elements = Vec::new();
                loop {
                    elements.push(self.parse_type());
                    if self.eat_punct(Punctuation::Comma) {
                        self.skip_newlines();
                        if self.edition == Edition::V0_2 && self.at_punct(Punctuation::RParen) {
                            break;
                        }
                        continue;
                    }
                    break;
                }
                self.expect_punct(Punctuation::RParen, "')'");
                Type {
                    kind: TypeKind::Tuple(elements),
                    span: self.span_from(start),
                }
            }
            TokenKind::Keyword(Keyword::Fn) => {
                self.bump();
                self.expect_punct(Punctuation::LParen, "'('");
                let parameters = self.parse_type_list_after_open();
                let (return_type, error_set) = self.parse_return_and_errors();

                Type {
                    kind: TypeKind::Function {
                        parameters,
                        return_type: return_type.map(Box::new),
                        error_set,
                    },
                    span: self.span_from(start),
                }
            }
            TokenKind::Identifier => Type {
                kind: TypeKind::Path(self.parse_type_path()),
                span: self.span_from(start),
            },
            TokenKind::Keyword(Keyword::Char) if self.edition == Edition::V0_2 => {
                self.bump();
                Type {
                    kind: TypeKind::Path(TypePath {
                        segments: vec!["char".to_owned()],
                        arguments: Vec::new(),
                        span: token.span,
                    }),
                    span: token.span,
                }
            }
            _ => {
                self.error(
                    INVALID_TYPE,
                    "Expected a type.",
                    token.span,
                    "Use a primitive, named, array, map, tuple, or function type.",
                );
                if !self.at_eof()
                    && !self.at(TokenKind::Newline)
                    && !self.at_punct(Punctuation::Semicolon)
                    && !self.at_punct(Punctuation::RBrace)
                    && !self.at_punct(Punctuation::RParen)
                    && !self.at_punct(Punctuation::RBracket)
                    && !self.at_punct(Punctuation::Comma)
                {
                    self.bump();
                }
                Type {
                    kind: TypeKind::Unit,
                    span: Span::new(start, self.last_end()),
                }
            }
        }
    }

    fn parse_type_path(&mut self) -> TypePath {
        let start = self.peek().span.start;
        let mut segments = Vec::new();

        if self.at(TokenKind::Identifier) {
            segments.push(self.bump().text);
        } else {
            self.error_current(
                INVALID_TYPE,
                "Expected a type name.",
                "Use an identifier such as i32, User, or Option<T>.",
            );
        }

        while self.eat_punct(Punctuation::Dot) {
            segments.push(self.expect_identifier().unwrap_or_default());
        }

        let arguments = if self.at_punct(Punctuation::Lt) {
            self.parse_type_arguments()
        } else {
            Vec::new()
        };

        TypePath {
            segments,
            arguments,
            span: self.span_from(start),
        }
    }

    fn parse_type_list_after_open(&mut self) -> Vec<Type> {
        let mut types = Vec::new();
        self.skip_newlines();

        if self.at_punct(Punctuation::RParen) {
            self.bump();
            return types;
        }

        loop {
            types.push(self.parse_type());
            if self.eat_punct(Punctuation::Comma) {
                self.skip_newlines();
                if self.at_punct(Punctuation::RParen) {
                    break;
                }
                continue;
            }
            break;
        }

        self.skip_newlines();
        self.expect_punct(Punctuation::RParen, "')'");
        types
    }

    fn parse_path(&mut self) -> Vec<String> {
        let mut path = Vec::new();

        if self.at(TokenKind::Identifier) {
            path.push(self.bump().text);
        } else {
            self.error_current(
                EXPECTED,
                "Expected a module path.",
                "Use a dotted path such as app.main.",
            );
            return path;
        }

        while self.eat_punct(Punctuation::Dot) {
            path.push(self.expect_identifier().unwrap_or_default());
        }

        path
    }

    fn parse_name_or_default(&mut self) -> String {
        if self.at_keyword(Keyword::Default) {
            self.bump().text
        } else {
            self.expect_identifier().unwrap_or_default()
        }
    }

    fn looks_like_script_block(&self) -> bool {
        self.at_punct(Punctuation::LBrace)
            && self
                .peek_at(1)
                .is_some_and(|token| token.kind == TokenKind::Punct(Punctuation::At))
            && self.peek_at(2).is_some_and(|token| token.text == "script")
            && self
                .peek_at(3)
                .is_some_and(|token| token.kind == TokenKind::Punct(Punctuation::Colon))
    }

    fn at_canonical_scope(&self) -> bool {
        if self.edition != Edition::V0_2 || !self.at_punct(Punctuation::At) {
            return false;
        }
        let mut index = self.index + 1;
        let Some(name) = self.tokens.get(index) else {
            return false;
        };
        if name.kind != TokenKind::Identifier || !matches!(name.text.as_str(), "mode" | "memory") {
            return false;
        }
        index += 1;
        if self.tokens.get(index).map(|token| &token.kind)
            != Some(&TokenKind::Punct(Punctuation::LParen))
        {
            return false;
        }
        index = self.skip_newlines_at(index + 1);
        if !self
            .tokens
            .get(index)
            .is_some_and(|token| token.kind == TokenKind::Identifier)
        {
            return false;
        }
        index = self.skip_newlines_at(index + 1);
        if self.tokens.get(index).map(|token| &token.kind)
            != Some(&TokenKind::Punct(Punctuation::RParen))
        {
            return false;
        }
        index = self.skip_newlines_at(index + 1);
        self.tokens.get(index).map(|token| &token.kind)
            == Some(&TokenKind::Punct(Punctuation::LBrace))
    }

    fn skip_continuation_newlines(&mut self) {
        if self.edition == Edition::V0_2 {
            self.skip_newlines();
        }
    }

    fn next_non_newline_is_punct(&self, punctuation: Punctuation) -> bool {
        let index = self.skip_newlines_at(self.index);
        self.tokens.get(index).map(|token| &token.kind) == Some(&TokenKind::Punct(punctuation))
    }

    fn skip_newlines_at(&self, mut index: usize) -> usize {
        while self
            .tokens
            .get(index)
            .is_some_and(|token| token.kind == TokenKind::Newline)
        {
            index += 1;
        }
        index
    }

    fn looks_like_scope_annotation(&self) -> bool {
        self.at_punct(Punctuation::LBrace)
            && self
                .peek_at(1)
                .is_some_and(|token| token.kind == TokenKind::Punct(Punctuation::At))
            && self
                .peek_at(2)
                .is_some_and(|token| token.kind == TokenKind::Identifier && token.text != "script")
            && self
                .peek_at(3)
                .is_some_and(|token| token.kind == TokenKind::Punct(Punctuation::Colon))
    }

    fn looks_like_struct_init(&self) -> bool {
        if !self.at_punct(Punctuation::LBrace) {
            return false;
        }
        let name = self.skip_newlines_at(self.index + 1);
        let colon = self.skip_newlines_at(name + 1);
        self.tokens
            .get(name)
            .is_some_and(|token| token.kind == TokenKind::Identifier)
            && self
                .tokens
                .get(colon)
                .is_some_and(|token| token.kind == TokenKind::Punct(Punctuation::Colon))
    }

    fn expect_identifier(&mut self) -> Option<String> {
        if self.at(TokenKind::Identifier) {
            Some(self.bump().text)
        } else {
            self.error_current(EXPECTED, "Expected an identifier.", "Use a valid name.");
            None
        }
    }

    fn expect_string(&mut self) -> String {
        if self.at(TokenKind::String) {
            let token = self.bump();
            match decode_string_literal(&token.text) {
                Ok(value) => value,
                Err(message) => {
                    self.error(INVALID, message, token.span, "Fix the string literal.");
                    String::new()
                }
            }
        } else {
            self.error_current(
                EXPECTED,
                "Expected a string module specifier.",
                "Use a quoted specifier such as \"./thing.ns\" or \"std:fs\".",
            );
            String::new()
        }
    }

    fn expect_integer(&mut self) -> String {
        if self.at(TokenKind::Integer) {
            self.bump().text
        } else {
            self.error_current(
                EXPECTED,
                "Expected an integer literal.",
                "Use a decimal, hexadecimal, or binary integer.",
            );
            String::new()
        }
    }

    fn expect_keyword(&mut self, keyword: Keyword, description: &str) -> Option<Token> {
        if self.at_keyword(keyword) {
            Some(self.bump())
        } else {
            self.error_current(
                EXPECTED,
                format!("Expected {description}."),
                format!("Add {}.", keyword.as_str()),
            );
            None
        }
    }

    fn expect_punct(&mut self, punctuation: Punctuation, description: &str) -> Option<Token> {
        if self.at_punct(punctuation) {
            Some(self.bump())
        } else {
            self.error_current(
                EXPECTED,
                format!("Expected {description}."),
                "Check the surrounding syntax.",
            );
            None
        }
    }

    fn at_keyword(&self, keyword: Keyword) -> bool {
        self.peek().kind == TokenKind::Keyword(keyword)
    }

    fn at_punct(&self, punctuation: Punctuation) -> bool {
        self.peek().kind == TokenKind::Punct(punctuation)
    }

    fn eat_keyword(&mut self, keyword: Keyword) -> bool {
        if self.at_keyword(keyword) {
            self.bump();
            true
        } else {
            false
        }
    }

    fn eat_punct(&mut self, punctuation: Punctuation) -> bool {
        if self.at_punct(punctuation) {
            self.bump();
            true
        } else {
            false
        }
    }

    fn at(&self, kind: TokenKind) -> bool {
        self.peek().kind == kind
    }

    fn at_eof(&self) -> bool {
        self.at(TokenKind::Eof)
    }

    fn peek(&self) -> &Token {
        let index = self.index.min(self.tokens.len().saturating_sub(1));
        &self.tokens[index]
    }

    fn peek_at(&self, offset: usize) -> Option<&Token> {
        self.tokens.get(self.index + offset)
    }

    fn bump(&mut self) -> Token {
        let token = self.peek().clone();
        if self.index < self.tokens.len().saturating_sub(1) {
            self.index += 1;
        }
        token
    }

    fn last_end(&self) -> usize {
        if self.index == 0 {
            self.peek().span.end
        } else {
            self.tokens[self.index - 1].span.end
        }
    }

    fn span_from(&self, start: usize) -> Span {
        Span::new(start, self.last_end())
    }

    fn skip_newlines(&mut self) {
        while self.at(TokenKind::Newline) {
            self.bump();
        }
    }

    fn skip_separators(&mut self) {
        while self.at(TokenKind::Newline) || self.at_punct(Punctuation::Semicolon) {
            self.bump();
        }
    }

    fn finish_boundary(&mut self, allow_rbrace: bool) {
        if self.at(TokenKind::Newline) || self.at_punct(Punctuation::Semicolon) {
            self.skip_separators();
            return;
        }

        if self.at_eof() || (allow_rbrace && self.at_punct(Punctuation::RBrace)) {
            return;
        }

        self.error_current(
            BOUNDARY,
            "Expected a newline or ';' between constructs.",
            "Put the next item or statement on a new line, or separate them with ';'.",
        );

        while !self.at_eof()
            && !self.at(TokenKind::Newline)
            && !self.at_punct(Punctuation::Semicolon)
            && !self.at_punct(Punctuation::RBrace)
        {
            self.bump();
        }
        self.skip_separators();
    }

    fn synchronize_until(&mut self, punctuation: &[Punctuation]) {
        while !self.at_eof() && !punctuation.iter().any(|item| self.at_punct(*item)) {
            self.bump();
        }
    }

    fn error(
        &mut self,
        code: &'static str,
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

    fn error_current(
        &mut self,
        code: &'static str,
        message: impl Into<String>,
        hint: impl Into<String>,
    ) {
        let span = self.peek().span;
        self.error(code, message, span, hint);
    }
}

fn is_binary_operator_token(kind: &TokenKind) -> bool {
    matches!(
        kind,
        TokenKind::Punct(
            Punctuation::Plus
                | Punctuation::Minus
                | Punctuation::Star
                | Punctuation::Slash
                | Punctuation::Percent
                | Punctuation::PlusWrap
                | Punctuation::MinusWrap
                | Punctuation::StarWrap
                | Punctuation::SlashWrap
                | Punctuation::EqEq
                | Punctuation::BangEq
                | Punctuation::Lt
                | Punctuation::LtEq
                | Punctuation::Gt
                | Punctuation::GtEq
                | Punctuation::AmpAmp
                | Punctuation::Amp
                | Punctuation::Pipe
                | Punctuation::Caret
                | Punctuation::PipePipe
                | Punctuation::Eq
        )
    )
}

fn expression_has_unparenthesized_struct_init(expression: &Expr) -> bool {
    match &expression.kind {
        ExprKind::StructInit { .. } => true,
        ExprKind::Binary { left, right, .. } => {
            expression_has_unparenthesized_struct_init(left)
                || expression_has_unparenthesized_struct_init(right)
        }
        ExprKind::Assign { target, value } => {
            expression_has_unparenthesized_struct_init(target)
                || expression_has_unparenthesized_struct_init(value)
        }
        ExprKind::Unary { operand, .. } => expression_has_unparenthesized_struct_init(operand),
        _ => false,
    }
}

fn expression_to_type_path(expression: &Expr) -> Option<TypePath> {
    match &expression.kind {
        ExprKind::Identifier(name) => Some(TypePath {
            segments: vec![name.clone()],
            arguments: Vec::new(),
            span: expression.span,
        }),
        ExprKind::Member { target, member } => {
            let mut path = expression_to_type_path(target)?;
            path.segments.push(member.clone());
            path.span = expression.span;
            Some(path)
        }
        ExprKind::TypeArguments { target, arguments } => {
            let mut path = expression_to_type_path(target)?;
            if !path.arguments.is_empty() {
                return None;
            }
            path.arguments = arguments.clone();
            path.span = expression.span;
            Some(path)
        }
        _ => None,
    }
}
