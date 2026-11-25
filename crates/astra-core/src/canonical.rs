use crate::diagnostic::Diagnostic;
use crate::edition::Edition;
use crate::lexer::{Keyword, LexOptions, Punctuation, TokenKind, lex_with_options};

const DEPRECATED_ALIAS: &str = "NS2009";
const DEPRECATED_SPAWN: &str = "NS2012";
const DEPRECATED_DIVISION: &str = "NS2013";
const DEPRECATED_SPELLING: &str = "NS2015";

const ALIASES: [(&str, &str); 7] = [
    ("function", "fn"),
    ("var", "let"),
    ("constant", "const"),
    ("typedef", "type"),
    ("record", "struct"),
    ("mutable", "mut"),
    ("public", "pub"),
];

#[must_use]
pub fn check_canonical(source: &str, edition: Edition) -> Vec<Diagnostic> {
    if edition != Edition::V0_2 {
        return Vec::new();
    }

    let tokens = lex_with_options(source, LexOptions { edition }).tokens;
    let mut diagnostics = Vec::new();
    let skip_newlines = |mut index: usize| {
        while tokens
            .get(index)
            .is_some_and(|token| token.kind == TokenKind::Newline)
        {
            index += 1;
        }
        index
    };

    for (index, token) in tokens.iter().enumerate() {
        if let TokenKind::Keyword(_) = token.kind
            && let Some((_, canonical)) = ALIASES
                .iter()
                .find(|(alias, _)| *alias == token.text.as_str())
        {
            diagnostics.push(
                Diagnostic::error(
                    DEPRECATED_ALIAS,
                    format!("Deprecated alias '{}'.", token.text),
                )
                .with_span(token.span)
                .with_hint(format!("Use '{canonical}' instead.")),
            );
            continue;
        }

        if token.kind == TokenKind::Punct(Punctuation::SlashWrap) {
            diagnostics.push(
                Diagnostic::error(
                    DEPRECATED_DIVISION,
                    "Wrapping division '/%' is not canonical.",
                )
                .with_span(token.span)
                .with_hint("Keep '/%'; migration never rewrites it to '/'."),
            );
            continue;
        }

        if token.kind == TokenKind::Keyword(Keyword::Spawn)
            && tokens
                .get(skip_newlines(index + 1))
                .is_some_and(|next| next.kind == TokenKind::Punct(Punctuation::PipePipe))
        {
            diagnostics.push(
                Diagnostic::error(DEPRECATED_SPAWN, "Deprecated 'spawn ||' form.")
                    .with_span(token.span)
                    .with_hint("Use 'spawn { ... }'."),
            );
            continue;
        }

        if token.kind == TokenKind::Punct(Punctuation::LBrace) {
            let mut cursor = skip_newlines(index + 1);
            let is_at = tokens
                .get(cursor)
                .is_some_and(|next| next.kind == TokenKind::Punct(Punctuation::At));
            if is_at {
                cursor = skip_newlines(cursor + 1);
                let has_name = tokens
                    .get(cursor)
                    .is_some_and(|next| next.kind == TokenKind::Identifier);
                if has_name {
                    cursor = skip_newlines(cursor + 1);
                    let has_colon = tokens
                        .get(cursor)
                        .is_some_and(|next| next.kind == TokenKind::Punct(Punctuation::Colon));
                    if has_colon {
                        diagnostics.push(
                            Diagnostic::error(
                                DEPRECATED_SPELLING,
                                "The old scope-block spelling is deprecated in edition 0.2.",
                            )
                            .with_span(token.span)
                            .with_hint("Use @mode(name) { ... } or @memory(name) { ... }."),
                        );
                    }
                }
            }
        }
    }

    diagnostics
}
