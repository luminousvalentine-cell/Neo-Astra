use crate::span::Span;

#[derive(Debug, Clone, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct Module {
    pub name: Vec<String>,
    pub attributes: Vec<Attribute>,
    pub items: Vec<Item>,

    pub header_span: Span,
    pub span: Span,
}

#[derive(
    Debug, Clone, Copy, Default, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize,
)]
pub enum Visibility {
    #[default]
    Normal,
    Public,
    Private,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct Attribute {
    pub name: String,
    pub arguments: Vec<Expr>,

    #[serde(default)]
    pub export_as: Option<ExportAsAttribute>,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct ExportAsAttribute {

    pub symbol: String,

    pub bindings: Vec<(String, Type)>,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct Item {
    pub attributes: Vec<Attribute>,
    pub visibility: Visibility,
    pub exported: bool,
    pub kind: ItemKind,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum ItemKind {
    Import(ImportDecl),
    Export(ExportDecl),
    Function(FunctionDecl),
    Struct(StructDecl),
    Enum(EnumDecl),
    Constant(ConstantDecl),
    TypeAlias(TypeAliasDecl),
    ForeignModule(ForeignModuleDecl),
    ScriptBlock(ScriptBlock),
    ScopedItems(ScopedItems),
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct ImportDecl {
    pub form: ImportForm,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum ImportForm {
    Path {
        path: Vec<String>,
        alias: Option<String>,
    },
    SideEffect {
        specifier: String,
    },
    Namespace {
        alias: String,
        specifier: String,
    },
    Named {
        specifiers: Vec<ImportSpecifier>,
        specifier: String,
    },
    Default {
        alias: String,
        specifier: String,
    },
    DefaultNamed {
        default: String,
        specifiers: Vec<ImportSpecifier>,
        specifier: String,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct ImportSpecifier {
    pub imported: String,
    pub local: String,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct ExportDecl {
    pub form: ExportForm,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum ExportForm {
    List {
        specifiers: Vec<ExportSpecifier>,
        from: Option<String>,
    },
    All {
        alias: Option<String>,
        from: String,
    },
    DefaultValue(Expr),
    DefaultFunction(FunctionDecl),
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct ExportSpecifier {
    pub local: String,
    pub exported: String,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct FunctionDecl {
    pub name: String,
    pub type_parameters: Vec<String>,
    pub parameters: Vec<Parameter>,
    pub return_type: Option<Type>,
    pub error_set: Vec<TypePath>,
    pub body: Block,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct Parameter {
    pub name: String,
    pub ty: Type,
    pub span: Span,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct Block {
    pub statements: Vec<Stmt>,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct StructDecl {
    pub name: String,
    pub type_parameters: Vec<String>,
    pub fields: StructFields,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum StructFields {
    Named(Vec<FieldDecl>),
    Tuple(Vec<Type>),
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct FieldDecl {
    pub name: String,
    pub ty: Type,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct EnumDecl {
    pub name: String,
    pub type_parameters: Vec<String>,
    pub variants: Vec<EnumVariant>,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct EnumVariant {
    pub name: String,
    pub fields: Option<Vec<VariantField>>,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct VariantField {
    pub name: Option<String>,
    pub ty: Type,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct ConstantDecl {
    pub name: String,
    pub ty: Type,
    pub value: Expr,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct TypeAliasDecl {
    pub name: String,
    pub type_parameters: Vec<String>,
    pub ty: Type,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct ForeignModuleDecl {
    pub name: String,
    pub functions: Vec<ForeignFunction>,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct ForeignFunction {
    pub name: String,
    pub parameters: Vec<Parameter>,
    pub return_type: Option<Type>,
    pub error_set: Vec<TypePath>,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct ScriptBlock {
    pub language: String,
    pub name: String,
    pub mode: String,
    pub exports: Vec<ScriptExport>,
    pub raw_source: String,
    pub span: Span,
    pub header_span: Span,
    pub body_span: Span,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct ScriptExport {
    pub name: String,
    pub parameters: Vec<Parameter>,
    pub return_type: Option<Type>,
    pub error_set: Vec<TypePath>,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct ScopedItems {
    pub annotation: ScopeAnnotation,
    pub items: Vec<Item>,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct ScopedStatements {
    pub annotation: ScopeAnnotation,
    pub statements: Vec<Stmt>,
    pub span: Span,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
pub enum ScopeKind {
    Mode,
    Memory,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct ScopeAnnotation {
    pub kind: ScopeKind,
    pub value: String,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct Type {
    pub kind: TypeKind,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum TypeKind {
    Path(TypePath),
    Array(Box<Type>),
    FixedArray(Box<Type>, String),
    Map(Box<Type>, Box<Type>),
    Tuple(Vec<Type>),
    Function {
        parameters: Vec<Type>,
        return_type: Option<Box<Type>>,
        error_set: Vec<TypePath>,
    },
    Unit,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct TypePath {
    pub segments: Vec<String>,
    pub arguments: Vec<Type>,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct Stmt {
    pub kind: StmtKind,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum StmtKind {
    Let {
        mutable: bool,
        name: String,
        ty: Option<Type>,
        value: Expr,
    },
    Return {
        value: Option<Expr>,
    },
    Break,
    Continue,
    Defer(Box<Stmt>),
    While {
        condition: Expr,
        body: Block,
    },
    For {
        binding: String,
        iterable: Expr,
        body: Block,
    },
    Expr(Expr),
    Scoped(ScopedStatements),
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct Expr {
    pub kind: ExprKind,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum ExprKind {
    Literal(Literal),
    Identifier(String),
    Unary {
        operator: UnaryOp,
        operand: Box<Expr>,
    },
    Binary {
        operator: BinaryOp,
        left: Box<Expr>,
        right: Box<Expr>,
    },
    Assign {
        target: Box<Expr>,
        value: Box<Expr>,
    },
    Call {
        callee: Box<Expr>,
        arguments: Vec<Expr>,
    },
    Index {
        target: Box<Expr>,
        index: Box<Expr>,
    },
    Member {
        target: Box<Expr>,
        member: String,
    },
    TypeArguments {
        target: Box<Expr>,
        arguments: Vec<Type>,
    },
    Propagate(Box<Expr>),
    Array(Vec<Expr>),
    Map(Vec<MapEntry>),
    Tuple(Vec<Expr>),
    StructInit {
        path: TypePath,
        fields: Vec<FieldInit>,
    },
    Block(Block),
    If {
        condition: Box<Expr>,
        then_branch: Block,
        else_branch: Option<ElseBranch>,
    },
    Match {
        value: Box<Expr>,
        arms: Vec<MatchArm>,
    },
    Spawn {
        body: Block,
    },
    Fail {
        error: TypePath,
        arguments: FailArguments,
    },
    Panic {
        message: Box<Expr>,
    },
    Abort {
        message: Box<Expr>,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum FailArguments {
    Tuple(Vec<Expr>),
    Struct(Vec<FieldInit>),
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct MapEntry {
    pub key: Expr,
    pub value: Expr,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct FieldInit {
    pub name: String,
    pub value: Expr,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct MatchArm {
    pub pattern: Pattern,
    pub body: MatchArmBody,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum MatchArmBody {
    Expr(Expr),
    Block(Block),
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum ElseBranch {
    Block(Block),
    If(Box<Expr>),
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct Pattern {
    pub kind: PatternKind,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum PatternKind {
    Wildcard,
    Identifier(String),
    Literal(Literal),
    Tuple(Vec<Pattern>),
    Constructor {
        path: TypePath,
        arguments: Vec<Pattern>,
    },
    Variant {
        path: TypePath,
        variant: String,
        arguments: Option<Vec<Pattern>>,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum Literal {
    Integer(String),
    Float(String),
    String(String),
    Bytes(Vec<u8>),
    Bool(bool),
    Unit,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
pub enum UnaryOp {
    Negate,
    Not,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
pub enum BinaryOp {
    Add,
    Subtract,
    Multiply,
    Divide,
    Remainder,
    AddWrap,
    SubtractWrap,
    MultiplyWrap,
    DivideWrap,
    Equal,
    NotEqual,
    Less,
    LessEqual,
    Greater,
    GreaterEqual,
    And,
    Or,
}
