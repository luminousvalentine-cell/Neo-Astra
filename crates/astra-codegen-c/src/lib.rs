mod cstrip;
mod emit;
pub mod runtime;
mod ty;

pub const TARGET: &str = "c";

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum IntegerPolicy {
    #[default]
    Wrapping,

    SafeChecked,

    Arbitrary,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CModule {
    pub name: String,

    pub ast: astra_core::ast::Module,

    pub hir: astra_core::hir::Module,

    pub annotations: Vec<astra_core::checker::TypeAnnotation>,

    pub manual_memory_scopes: Vec<astra_core::span::Span>,

    pub integer_policy: IntegerPolicy,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BackendError {
    pub code: String,

    pub message: String,
}

impl BackendError {
    #[must_use]
    pub fn new(code: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            code: code.into(),
            message: message.into(),
        }
    }

    #[must_use]
    pub fn unsupported() -> Self {
        Self::new("NS8001", "The C backend does not support this request.")
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GeneratedC {
    pub source: String,

    pub header: Option<String>,

    pub host_shim: String,

    pub script_sources: Vec<(String, String)>,

    pub target: String,
}

pub fn generate(module: &CModule) -> Result<GeneratedC, BackendError> {
    emit::emit(module).map(strip_generated)
}

fn strip_generated(mut generated: GeneratedC) -> GeneratedC {
    generated.source = cstrip::strip_comments(&generated.source);
    generated.header = generated
        .header
        .map(|header| cstrip::strip_comments(&header));
    generated.host_shim = cstrip::strip_comments(&generated.host_shim);
    generated.script_sources = generated
        .script_sources
        .into_iter()
        .map(|(name, source)| (name, cstrip::strip_comments(&source)))
        .collect();
    generated
}

#[derive(Debug, Clone)]
pub struct ProjectEntry {
    pub module: CModule,

    pub key: String,

    pub dep_keys: Vec<String>,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum EmitKind {
    #[default]
    Executable,

    Library,
}

pub fn generate_project(entries: &[ProjectEntry]) -> Result<Vec<GeneratedC>, BackendError> {
    emit::emit_project(entries, true)
        .map(|generated| generated.into_iter().map(strip_generated).collect())
}

pub fn generate_project_with_kind(
    entries: &[ProjectEntry],
    kind: EmitKind,
) -> Result<Vec<GeneratedC>, BackendError> {
    emit::emit_project(entries, kind == EmitKind::Executable)
        .map(|generated| generated.into_iter().map(strip_generated).collect())
}
