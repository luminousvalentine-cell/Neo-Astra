use std::collections::BTreeMap;

use crate::ast;
use crate::span::Span;
use crate::types::Type;

#[derive(Debug, Clone, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct Module {
    pub name: String,

    pub items: Vec<Item>,

    pub imports: Vec<Import>,

    pub symbols: SymbolTable,

    pub exports: ExportSurface,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct Item {
    pub name: String,
    pub kind: ItemKind,

    pub ty: Option<Type>,
    pub span: Span,
    pub visibility: ast::Visibility,

    pub exported: bool,

    pub declaration: Declaration,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
pub enum ItemKind {
    Function,
    Struct,
    Enum,
    Constant,
    TypeAlias,
    ForeignModule,
    ScriptBlock,
}

impl ItemKind {
    #[must_use]
    pub const fn export_kind(self) -> ExportKind {
        match self {
            Self::Function | Self::Constant => ExportKind::Value,
            Self::Struct | Self::Enum => ExportKind::ValueAndType,
            Self::TypeAlias => ExportKind::Type,
            Self::ForeignModule | Self::ScriptBlock => ExportKind::Namespace,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum Declaration {
    Function(ast::FunctionDecl),
    Struct(ast::StructDecl),
    Enum(ast::EnumDecl),
    Constant(ast::ConstantDecl),
    TypeAlias(ast::TypeAliasDecl),
    ForeignModule(ast::ForeignModuleDecl),
    ScriptBlock(ast::ScriptBlock),
}

impl Declaration {
    #[must_use]
    pub const fn kind(&self) -> ItemKind {
        match self {
            Self::Function(_) => ItemKind::Function,
            Self::Struct(_) => ItemKind::Struct,
            Self::Enum(_) => ItemKind::Enum,
            Self::Constant(_) => ItemKind::Constant,
            Self::TypeAlias(_) => ItemKind::TypeAlias,
            Self::ForeignModule(_) => ItemKind::ForeignModule,
            Self::ScriptBlock(_) => ItemKind::ScriptBlock,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct Import {
    pub module: String,
    pub kind: ImportKind,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum ImportKind {
    SideEffect,
    Namespace {
        alias: String,
    },
    Named {
        bindings: Vec<ImportBinding>,
    },
    Default {
        local: String,
    },
    DefaultNamed {
        default: String,
        bindings: Vec<ImportBinding>,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct ImportBinding {
    pub imported: String,
    pub local: String,
    pub span: Span,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct SymbolTable {
    pub values: BTreeMap<String, Symbol>,
    pub types: BTreeMap<String, Symbol>,
    pub namespaces: BTreeMap<String, Symbol>,
}

impl SymbolTable {
    #[must_use]
    pub fn get(&self, name: &str) -> Option<&Symbol> {
        self.values
            .get(name)
            .or_else(|| self.types.get(name))
            .or_else(|| self.namespaces.get(name))
    }

    #[must_use]
    pub fn contains_name(&self, name: &str) -> bool {
        self.values.contains_key(name)
            || self.types.contains_key(name)
            || self.namespaces.contains_key(name)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct Symbol {
    pub name: String,
    pub kind: SymbolKind,
    pub visibility: ast::Visibility,

    pub exported: bool,
    pub span: Span,
    pub origin: SymbolOrigin,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
pub enum SymbolKind {
    Function,
    Struct,
    Enum,
    Constant,
    TypeAlias,
    ForeignModule,
    ScriptBlock,
    ImportedValue,
    ImportedType,
    ImportedNamespace,
}

impl SymbolKind {
    #[must_use]
    pub const fn export_kind(self) -> ExportKind {
        match self {
            Self::Function | Self::Constant | Self::ImportedValue => ExportKind::Value,
            Self::Struct | Self::Enum => ExportKind::ValueAndType,
            Self::TypeAlias | Self::ImportedType => ExportKind::Type,
            Self::ForeignModule | Self::ImportedNamespace | Self::ScriptBlock => {
                ExportKind::Namespace
            }
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum SymbolOrigin {
    Local,
    Imported { module: String, name: String },
}

#[derive(Debug, Clone, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct ExportSurface {
    pub named: BTreeMap<String, Export>,

    pub default: Option<Export>,

    pub stars: Vec<StarExport>,
}

impl ExportSurface {
    #[must_use]
    pub fn get_named(&self, name: &str) -> Option<&Export> {
        self.named.get(name)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct StarExport {
    pub module: String,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct Export {
    pub name: String,
    pub kind: ExportKind,
    pub target: ExportTarget,
    pub span: Span,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
pub enum ExportKind {
    Value,
    Type,
    ValueAndType,
    Namespace,
}

impl ExportKind {
    #[must_use]
    pub const fn is_value(self) -> bool {
        matches!(self, Self::Value | Self::ValueAndType | Self::Namespace)
    }

    #[must_use]
    pub const fn is_type(self) -> bool {
        matches!(self, Self::Type | Self::ValueAndType)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum ExportTarget {
    Local(String),

    Reexport { module: String, name: String },

    Namespace { module: String },

    Expression,
}
