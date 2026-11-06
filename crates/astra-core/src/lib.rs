pub mod ast;
pub mod checker;
pub mod diagnostic;
pub mod formatter;
pub mod free_names;
pub mod hir;
pub mod ir;
pub mod lexer;
pub mod literal;
pub mod mode;
pub mod parser;
pub mod resolver;
pub mod span;
pub mod type_surface;
pub mod types;

pub const VERSION: &str = env!("CARGO_PKG_VERSION");
