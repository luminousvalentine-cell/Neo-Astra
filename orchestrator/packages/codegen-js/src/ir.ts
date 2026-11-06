export interface Span {
  readonly start: number;
  readonly end: number;
}

export interface TypePath {
  readonly segments: readonly string[];
  readonly arguments: readonly AstType[];
  readonly span: Span;
}

export interface AstType {
  readonly kind: AstTypeKind;
  readonly span: Span;
}

export type AstTypeKind =
  | "Unit"
  | { readonly Path: TypePath }
  | { readonly Array: AstType }
  | { readonly FixedArray: readonly [AstType, string] }
  | { readonly Map: readonly [AstType, AstType] }
  | { readonly Tuple: readonly AstType[] }
  | {
      readonly Function: {
        readonly parameters: readonly AstType[];
        readonly return_type: AstType | null;
        readonly error_set: readonly TypePath[];
      };
    };

export type SemanticType =
  | "I8"
  | "I16"
  | "I32"
  | "I64"
  | "U8"
  | "U16"
  | "U32"
  | "U64"
  | "Usize"
  | "Isize"
  | "F32"
  | "F64"
  | "Bool"
  | "Str"
  | "Bytes"
  | "Unit"
  | "Never"
  | "Error"
  | "Unknown"
  | { readonly Option: SemanticType }
  | { readonly Result: readonly [SemanticType, SemanticType] }
  | { readonly Array: SemanticType }
  | { readonly FixedArray: readonly [SemanticType, number] }
  | { readonly Slice: SemanticType }
  | { readonly Map: readonly [SemanticType, SemanticType] }
  | { readonly Tuple: readonly SemanticType[] }
  | { readonly Function: FunctionType }
  | { readonly Named: readonly [string, readonly SemanticType[]] }
  | { readonly Generic: string };

export interface FunctionType {
  readonly type_parameters: readonly string[];
  readonly parameters: readonly SemanticType[];
  readonly return_type: SemanticType;
  readonly errors: readonly SemanticType[];
}

export interface AstModule {
  readonly name: readonly string[];
  readonly attributes: readonly Attribute[];
  readonly items: readonly AstItem[];
  readonly header_span: Span;
  readonly span: Span;
}

export interface Attribute {
  readonly name: string;
  readonly arguments: readonly Expr[];
  readonly span: Span;
}

export type Visibility = "Normal" | "Public" | "Private";

export interface AstItem {
  readonly attributes: readonly Attribute[];
  readonly visibility: Visibility;
  readonly exported: boolean;
  readonly kind: AstItemKind;
  readonly span: Span;
}

export type AstItemKind =
  | { readonly Import: ImportDecl }
  | { readonly Export: ExportDecl }
  | { readonly Function: FunctionDecl }
  | { readonly Struct: StructDecl }
  | { readonly Enum: EnumDecl }
  | { readonly Constant: ConstantDecl }
  | { readonly TypeAlias: TypeAliasDecl }
  | { readonly ForeignModule: ForeignModuleDecl }
  | { readonly ScriptBlock: ScriptBlockDecl }
  | { readonly ScopedItems: ScopedItems };

export interface ImportDecl {
  readonly form: ImportForm;
  readonly span: Span;
}

export type ImportForm =
  | { readonly Path: { readonly path: readonly string[]; readonly alias: string | null } }
  | { readonly SideEffect: { readonly specifier: string } }
  | { readonly Namespace: { readonly alias: string; readonly specifier: string } }
  | {
      readonly Named: {
        readonly specifiers: readonly ImportSpecifier[];
        readonly specifier: string;
      };
    }
  | { readonly Default: { readonly alias: string; readonly specifier: string } }
  | {
      readonly DefaultNamed: {
        readonly default: string;
        readonly specifiers: readonly ImportSpecifier[];
        readonly specifier: string;
      };
    };

export interface ImportSpecifier {
  readonly imported: string;
  readonly local: string;
  readonly span: Span;
}

export interface ExportDecl {
  readonly form: ExportForm;
  readonly span: Span;
}

export type ExportForm =
  | {
      readonly List: {
        readonly specifiers: readonly ExportSpecifier[];
        readonly from: string | null;
      };
    }
  | { readonly All: { readonly alias: string | null; readonly from: string } }
  | { readonly DefaultValue: Expr }
  | { readonly DefaultFunction: FunctionDecl };

export interface ExportSpecifier {
  readonly local: string;
  readonly exported: string;
  readonly span: Span;
}

export interface FunctionDecl {
  readonly name: string;
  readonly type_parameters: readonly string[];
  readonly parameters: readonly Parameter[];
  readonly return_type: AstType | null;
  readonly error_set: readonly TypePath[];
  readonly body: Block;
  readonly span: Span;
}

export interface Parameter {
  readonly name: string;
  readonly ty: AstType;
  readonly span: Span;
}

export interface Block {
  readonly statements: readonly Stmt[];
  readonly span: Span;
}

export interface StructDecl {
  readonly name: string;
  readonly type_parameters: readonly string[];
  readonly fields: StructFields;
  readonly span: Span;
}

export type StructFields =
  | { readonly Named: readonly FieldDecl[] }
  | { readonly Tuple: readonly AstType[] };

export interface FieldDecl {
  readonly name: string;
  readonly ty: AstType;
  readonly span: Span;
}

export interface EnumDecl {
  readonly name: string;
  readonly type_parameters: readonly string[];
  readonly variants: readonly EnumVariant[];
  readonly span: Span;
}

export interface EnumVariant {
  readonly name: string;
  readonly fields: readonly VariantField[] | null;
  readonly span: Span;
}

export interface VariantField {
  readonly name: string | null;
  readonly ty: AstType;
  readonly span: Span;
}

export interface ConstantDecl {
  readonly name: string;
  readonly ty: AstType;
  readonly value: Expr;
  readonly span: Span;
}

export interface TypeAliasDecl {
  readonly name: string;
  readonly type_parameters: readonly string[];
  readonly ty: AstType;
  readonly span: Span;
}

export interface ForeignModuleDecl {
  readonly name: string;
  readonly functions: readonly ForeignFunction[];
  readonly span: Span;
}

export interface ScriptBlockDecl {
  readonly language: string;
  readonly name: string;
  readonly mode: string;
  readonly exports: readonly ScriptExport[];
  readonly raw_source: string;
  readonly span: Span;
  readonly header_span: Span;
  readonly body_span: Span;
}

export interface ScriptExport {
  readonly name: string;
  readonly parameters: readonly Parameter[];
  readonly return_type: AstType | null;
  readonly error_set: readonly TypePath[];
  readonly span: Span;
}

export interface ForeignFunction {
  readonly name: string;
  readonly parameters: readonly Parameter[];
  readonly return_type: AstType | null;
  readonly error_set: readonly TypePath[];
  readonly span: Span;
}

export interface ScopedItems {
  readonly annotation: ScopeAnnotation;
  readonly items: readonly AstItem[];
  readonly span: Span;
}

export interface ScopedStatements {
  readonly annotation: ScopeAnnotation;
  readonly statements: readonly Stmt[];
  readonly span: Span;
}

export interface ScopeAnnotation {
  readonly kind: "Mode" | "Memory";
  readonly value: string;
  readonly span: Span;
}

export interface Stmt {
  readonly kind: StmtKind;
  readonly span: Span;
}

export type StmtKind =
  | {
      readonly Let: {
        readonly mutable: boolean;
        readonly name: string;
        readonly ty: AstType | null;
        readonly value: Expr;
      };
    }
  | { readonly Return: { readonly value: Expr | null } }
  | "Break"
  | "Continue"
  | { readonly Defer: Stmt }
  | {
      readonly While: {
        readonly condition: Expr;
        readonly body: Block;
      };
    }
  | {
      readonly For: {
        readonly binding: string;
        readonly iterable: Expr;
        readonly body: Block;
      };
    }
  | { readonly Expr: Expr }
  | { readonly Scoped: ScopedStatements };

export interface Expr {
  readonly kind: ExprKind;
  readonly span: Span;
}

export type ExprKind =
  | { readonly Literal: Literal }
  | { readonly Identifier: string }
  | {
      readonly Unary: {
        readonly operator: "Negate" | "Not";
        readonly operand: Expr;
      };
    }
  | {
      readonly Binary: {
        readonly operator: BinaryOp;
        readonly left: Expr;
        readonly right: Expr;
      };
    }
  | {
      readonly Assign: {
        readonly target: Expr;
        readonly value: Expr;
      };
    }
  | {
      readonly Call: {
        readonly callee: Expr;
        readonly arguments: readonly Expr[];
      };
    }
  | {
      readonly Index: {
        readonly target: Expr;
        readonly index: Expr;
      };
    }
  | {
      readonly Member: {
        readonly target: Expr;
        readonly member: string;
      };
    }
  | {
      readonly TypeArguments: {
        readonly target: Expr;
        readonly arguments: readonly AstType[];
      };
    }
  | { readonly Propagate: Expr }
  | { readonly Array: readonly Expr[] }
  | { readonly Map: readonly MapEntry[] }
  | { readonly Tuple: readonly Expr[] }
  | {
      readonly StructInit: {
        readonly path: TypePath;
        readonly fields: readonly FieldInit[];
      };
    }
  | { readonly Block: Block }
  | {
      readonly If: {
        readonly condition: Expr;
        readonly then_branch: Block;
        readonly else_branch: ElseBranch | null;
      };
    }
  | {
      readonly Match: {
        readonly value: Expr;
        readonly arms: readonly MatchArm[];
      };
    }
  | { readonly Spawn: { readonly body: Block } }
  | {
      readonly Fail: {
        readonly error: TypePath;
        readonly arguments: FailArguments;
      };
    }
  | { readonly Panic: { readonly message: Expr } }
  | { readonly Abort: { readonly message: Expr } };

export type FailArguments =
  | { readonly Tuple: readonly Expr[] }
  | { readonly Struct: readonly FieldInit[] };

export type BinaryOp =
  | "Add"
  | "Subtract"
  | "Multiply"
  | "Divide"
  | "Remainder"
  | "AddWrap"
  | "SubtractWrap"
  | "MultiplyWrap"
  | "DivideWrap"
  | "Equal"
  | "NotEqual"
  | "Less"
  | "LessEqual"
  | "Greater"
  | "GreaterEqual"
  | "And"
  | "Or";

export interface MapEntry {
  readonly key: Expr;
  readonly value: Expr;
  readonly span: Span;
}

export interface FieldInit {
  readonly name: string;
  readonly value: Expr;
  readonly span: Span;
}

export interface MatchArm {
  readonly pattern: Pattern;
  readonly body: MatchArmBody;
  readonly span: Span;
}

export type MatchArmBody =
  | { readonly Expr: Expr }
  | { readonly Block: Block };

export type ElseBranch =
  | { readonly Block: Block }
  | { readonly If: Expr };

export interface Pattern {
  readonly kind: PatternKind;
  readonly span: Span;
}

export type PatternKind =
  | "Wildcard"
  | { readonly Identifier: string }
  | { readonly Literal: Literal }
  | { readonly Tuple: readonly Pattern[] }
  | {
      readonly Constructor: {
        readonly path: TypePath;
        readonly arguments: readonly Pattern[];
      };
    }
  | {
      readonly Variant: {
        readonly path: TypePath;
        readonly variant: string;
        readonly arguments: readonly Pattern[] | null;
      };
    };

export type Literal =
  | { readonly Integer: string }
  | { readonly Float: string }
  | { readonly String: string }
  | { readonly Bytes: readonly number[] }
  | { readonly Bool: boolean }
  | "Unit";

export interface HirModule {
  readonly name: string;
  readonly items: readonly HirItem[];
  readonly imports: readonly HirImport[];
  readonly symbols: SymbolTable;
  readonly exports: ExportSurface;
}

export interface HirItem {
  readonly name: string;
  readonly kind:
    | "Function"
    | "Struct"
    | "Enum"
    | "Constant"
    | "TypeAlias"
    | "ForeignModule"
    | "ScriptBlock";
  readonly ty: SemanticType | null;
  readonly span: Span;
  readonly visibility: Visibility;
  readonly exported: boolean;
  readonly declaration: Declaration;
}

export type Declaration =
  | { readonly Function: FunctionDecl }
  | { readonly Struct: StructDecl }
  | { readonly Enum: EnumDecl }
  | { readonly Constant: ConstantDecl }
  | { readonly TypeAlias: TypeAliasDecl }
  | { readonly ForeignModule: ForeignModuleDecl }
  | { readonly ScriptBlock: ScriptBlockDecl };

export interface SymbolTable {
  readonly values: Readonly<Record<string, Symbol>>;
  readonly types: Readonly<Record<string, Symbol>>;
  readonly namespaces: Readonly<Record<string, Symbol>>;
}

export interface Symbol {
  readonly name: string;
  readonly kind: string;
  readonly visibility: Visibility;
  readonly exported: boolean;
  readonly span: Span;
  readonly origin: unknown;
}

export interface HirImport {
  readonly module: string;
  readonly kind: ImportKind;
  readonly span: Span;
}

export type ImportKind =
  | "SideEffect"
  | { readonly Namespace: { readonly alias: string } }
  | { readonly Named: { readonly bindings: readonly ImportBinding[] } }
  | { readonly Default: { readonly local: string } }
  | {
      readonly DefaultNamed: {
        readonly default: string;
        readonly bindings: readonly ImportBinding[];
      };
    };

export interface ImportBinding {
  readonly imported: string;
  readonly local: string;
  readonly span: Span;
}

export interface ExportSurface {
  readonly named: Readonly<Record<string, Export>>;
  readonly default: Export | null;
  readonly stars: readonly StarExport[];
}

export interface StarExport {
  readonly module: string;
  readonly span: Span;
}

export interface Export {
  readonly name: string;
  readonly kind: "Value" | "Type" | "ValueAndType" | "Namespace";
  readonly target: ExportTarget;
  readonly span: Span;
}

export type ExportTarget =
  | { readonly Local: string }
  | { readonly Reexport: { readonly module: string; readonly name: string } }
  | { readonly Namespace: { readonly module: string } }
  | "Expression";

export interface TypeAnnotationIr {
  readonly span: Span;
  readonly ty: SemanticType;
}

export interface ModuleIr {
  readonly key: string;
  readonly path: string;
  readonly ast: unknown;
  readonly hir: unknown;
  readonly annotations: readonly TypeAnnotationIr[];
  readonly manualMemoryScopes: readonly Span[];
  readonly integerPolicy: "wrapping" | "safe-checked" | "arbitrary";
}

export function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}

export function variantOf(value: unknown): string {
  if (typeof value === "string") {
    return value;
  }
  if (isRecord(value)) {
    const keys = Object.keys(value);
    if (keys.length === 1 && keys[0] !== undefined) {
      return keys[0];
    }
  }
  throw new TypeError("Expected a serialized enum value.");
}

export function payloadOf(value: unknown): unknown {
  if (typeof value === "string") {
    return undefined;
  }
  if (isRecord(value)) {
    const keys = Object.keys(value);
    if (keys.length === 1 && keys[0] !== undefined) {
      return value[keys[0]];
    }
    return value;
  }
  return undefined;
}

export function structPayload<T>(value: unknown, expected: string): T {
  if (variantOf(value) !== expected) {
    throw new TypeError(`Expected '${expected}' variant.`);
  }
  const payload = payloadOf(value);
  if (!isRecord(payload)) {
    throw new TypeError(`Expected an object payload for '${expected}'.`);
  }
  return payload as unknown as T;
}

export function typeTag(type: SemanticType): string {
  return variantOf(type);
}

export function typePayload(type: SemanticType): unknown {
  return payloadOf(type);
}

export function sequenceElement(type: SemanticType): SemanticType | undefined {
  const tag = typeTag(type);
  if (tag === "Array" || tag === "Slice") {
    return typePayload(type) as SemanticType;
  }
  if (tag === "FixedArray") {
    const payload = typePayload(type) as readonly [SemanticType, number];
    return payload[0];
  }
  return undefined;
}
