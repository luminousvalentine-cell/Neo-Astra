use std::fmt;

use serde::{Deserialize, Serialize};

use crate::span::Span;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Severity {
    Error,
    Warning,
    Note,
    Help,
}

impl Severity {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Error => "error",
            Self::Warning => "warning",
            Self::Note => "note",
            Self::Help => "help",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RelatedSpan {
    pub file: String,
    pub span: Span,
    pub message: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Diagnostic {
    pub code: String,
    pub severity: Severity,
    pub message: String,
    pub span: Option<Span>,
    pub hint: Option<String>,

    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub related: Vec<RelatedSpan>,
}

impl Diagnostic {
    #[must_use]
    pub fn error(code: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            code: code.into(),
            severity: Severity::Error,
            message: message.into(),
            span: None,
            hint: None,
            related: Vec::new(),
        }
    }

    #[must_use]
    pub fn with_span(mut self, span: Span) -> Self {
        self.span = Some(span);
        self
    }

    #[must_use]
    pub fn with_hint(mut self, hint: impl Into<String>) -> Self {
        self.hint = Some(hint.into());
        self
    }

    #[must_use]
    pub fn with_related(
        mut self,
        file: impl Into<String>,
        span: Span,
        message: impl Into<String>,
    ) -> Self {
        self.related.push(RelatedSpan {
            file: file.into(),
            span,
            message: message.into(),
        });
        self
    }
}

impl fmt::Display for Diagnostic {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{}: {}", self.code, self.message)
    }
}

impl std::error::Error for Diagnostic {}
