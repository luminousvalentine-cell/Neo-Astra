import {
  type AstItem,
  type AstModule,
  type AstType,
  type Block,
  type EnumDecl,
  type Expr,
  type FunctionType,
  type HirItem,
  type HirModule,
  type ImportBinding,
  type Literal,
  type MatchArm,
  type ModuleIr,
  type Pattern,
  type ScriptBlockDecl,
  type ScriptExport,
  type SemanticType,
  type Span,
  type Stmt,
  type StructDecl,
  type TypeAliasDecl,
  type TypePath,
  isRecord,
  payloadOf,
  structPayload,
  typePayload,
  typeTag,
  variantOf,
} from "./ir.js";

export interface CodegenDiagnostic {
  readonly code: string;
  readonly message: string;
  readonly span?: Span;
}

export interface CodegenRequest {
  readonly moduleName: string;
  readonly ir: unknown;
  readonly sourceText?: string;

  readonly moduleSpecifiers?: Readonly<Record<string, string>>;

  readonly backend?: "js" | "ts";

  readonly transpileTypeScript?: TypeScriptTranspiler;
}

export type TypeScriptTranspileOutcome =
  | { readonly ok: true; readonly code: string; readonly sourceMap?: string }
  | { readonly ok: false; readonly message: string };

export type TypeScriptTranspiler = (source: string) => TypeScriptTranspileOutcome;

export interface CodegenArtifact {

  readonly path: string;
  readonly code: string;
  readonly sourceMap?: string;
  readonly declarations?: string;
}

export interface CodegenResult {
  readonly code: string;
  readonly diagnostics: readonly CodegenDiagnostic[];
  readonly sourceMap?: string;
  readonly declarations?: string;
  readonly artifacts?: readonly CodegenArtifact[];
}

export class CodegenRequestError extends Error {
  readonly code = "NS8002";

  constructor(message: string) {
    super(message);
    this.name = "CodegenRequestError";
  }
}

const JS_RESERVED = new Set([
  "await",
  "break",
  "case",
  "catch",
  "class",
  "const",
  "continue",
  "debugger",
  "default",
  "delete",
  "do",
  "else",
  "enum",
  "export",
  "extends",
  "false",
  "finally",
  "for",
  "function",
  "if",
  "implements",
  "import",
  "in",
  "instanceof",
  "interface",
  "let",
  "new",
  "null",
  "package",
  "private",
  "protected",
  "public",
  "return",
  "static",
  "super",
  "switch",
  "this",
  "throw",
  "true",
  "try",
  "typeof",
  "undefined",
  "var",
  "void",
  "while",
  "with",
  "yield",
]);

const IDENTIFIER = /^[A-Za-z_$][A-Za-z0-9_$]*$/;

const HELPER_ORDER = [
  "panic",
  "abort",
  "failure",
  "fail",
  "taskError",
  "taskScope",
  "spawn",
  "taskJoin",
  "taskCancel",
  "taskCanceled",
  "channel",
  "renderFloat",
  "render",
  "renderDebug",
  "float",
  "write",
  "print",
  "println",
  "dbg",
  "assert",
  "utf8Len",
  "len",
  "lenBig",
  "strAt",
  "charCount",
  "strBytes",
  "strChars",
  "byteAt",
  "charAt",
  "strSlice",
  "bytesNew",
  "byteSlice",
  "index",
  "indexSet",
  "getArray",
  "tuple",
  "tupleAt",
  "map",
  "mapGet",
  "mapInsert",
  "mapKeys",
  "eq",
  "mapIndex",
  "mapIndexSet",
  "none",
  "some",
  "ok",
  "err",
  "unwrapOption",
  "unwrapResult",
  "resultError",
  "parseInt",
  "parseIntBig",
  "div",
  "rem",
  "divBig",
  "remBig",
  "i8",
  "i16",
  "i32",
  "u8",
  "u16",
  "u32",
  "i64",
  "u64",
  "checked_i8",
  "checked_i16",
  "checked_i32",
  "checked_u8",
  "checked_u16",
  "checked_u32",
  "checked_i64",
  "checked_u64",
  "checkedInteger",
] as const;

type HelperName = (typeof HELPER_ORDER)[number];

const HELPER_DEPENDENCIES: Partial<Record<HelperName, readonly HelperName[]>> = {
  fail: ["failure"],
  taskError: ["panic"],
  taskScope: ["taskError"],
  spawn: ["taskScope", "taskJoin", "taskCancel", "taskCanceled"],
  taskJoin: ["failure", "taskError"],
  render: ["renderFloat"],
  print: ["write", "render"],
  println: ["write", "render"],
  dbg: ["render"],
  assert: ["panic"],
  len: ["panic", "utf8Len"],
  byteAt: ["some", "none"],
  charAt: ["some", "none"],
  strSlice: ["ok", "err"],
  byteSlice: ["panic"],
  indexSet: ["panic"],
  channel: ["ok", "err", "panic"],
  lenBig: ["panic"],
  strAt: ["panic"],
  index: ["panic"],
  mapGet: ["eq", "none"],
  mapInsert: ["eq", "none", "some"],
  mapIndex: ["eq", "panic"],
  mapIndexSet: ["eq"],
  mapKeys: [],
  getArray: ["none"],
  parseInt: ["ok", "err"],
  parseIntBig: ["ok", "err"],
  div: ["panic"],
  rem: ["panic"],
  divBig: ["panic"],
  remBig: ["panic"],
  checked_i8: ["panic"],
  checked_i16: ["panic"],
  checked_i32: ["panic"],
  checked_u8: ["panic"],
  checked_u16: ["panic"],
  checked_u32: ["panic"],
  checked_i64: ["panic"],
  checked_u64: ["panic"],
  checkedInteger: ["ok", "err", "panic"],
  unwrapResult: ["panic"],
  resultError: ["panic"],
  unwrapOption: ["panic"],
};

const HELPERS: Record<HelperName, readonly string[]> = {
  panic: [
    "function $panic(message) {",
    '  throw new Error("panic: " + message);',
    "}",
  ],
  abort: [
    "function $abort(message) {",
    "  const runtime = globalThis;",
    "  if (runtime.process !== undefined && typeof runtime.process.exit === \"function\") {",
    "    if (runtime.process.stderr && typeof runtime.process.stderr.write === \"function\") {",
    "      runtime.process.stderr.write(\"abort: \" + message + \"\\n\");",
    "    }",
    "    runtime.process.exit(134);",
    "  }",
    "  if (typeof runtime.close === \"function\") runtime.close();",
    "  throw new Error(\"abort: \" + message);",
    "}",
  ],
  failure: [
    "function $failure(error) {",
    "  return { $failure: true, error };",
    "}",
  ],
  fail: [
    "function $fail(error) {",
    "  throw $failure(error);",
    "}",
  ],
  taskError: [
    "function $taskError(value) {",
    '  let text = value instanceof Error ? value.message : String(value);',
    '  if (text.startsWith("panic: ")) text = text.slice(7);',
    '  $panic(text.startsWith("TaskError: ") ? text : "TaskError: " + text);',
    "}",
  ],
  taskScope: [
    "const $taskScopes = [];",
    "function $enterTaskScope() {",
    "  const scope = { tasks: [] };",
    "  $taskScopes.push(scope);",
    "  return scope;",
    "}",
    "function $exitTaskScope(scope) {",
    "  $taskScopes.pop();",
    "  for (const task of scope.tasks) {",
    "    if (task.joined) continue;",
    "    if (task.outcome === undefined) {",
    '      if (task.cancelled) $taskError("task was cancelled");',
    '      $taskError("task failed without a value");',
    "    }",
    "    if (task.outcome.panic !== undefined) $taskError(task.outcome.panic);",
    "  }",
    "}",
  ],
  spawn: [
    "function $spawn(body) {",
    "  const task = {",
    "    $task: true,",
    "    cancelled: false,",
    "    joined: false,",
    "    outcome: undefined,",
    "    join() { return $taskJoin(this); },",
    "    cancel() { return $taskCancel(this); },",
    "    canceled() { return $taskCanceled(this); },",
    "  };",
    "  try {",
    "    task.outcome = { ok: true, value: body() };",
    "  } catch (error) {",
    "    if (error && error.$failure) task.outcome = { ok: false, error: error.error };",
    "    else task.outcome = { panic: error };",
    "  }",
    "  if ($taskScopes.length > 0) $taskScopes[$taskScopes.length - 1].tasks.push(task);",
    "  return task;",
    "}",
  ],
  taskJoin: [
    "function $taskJoin(task) {",
    "  task.joined = true;",
    "  if (task.outcome === undefined) {",
    '    if (task.cancelled) $taskError("task was cancelled");',
    '    $taskError("task failed without a value");',
    "  }",
    "  if (task.outcome.panic !== undefined) $taskError(task.outcome.panic);",
    "  if (!task.outcome.ok) return $failure(task.outcome.error);",
    "  return task.outcome.value;",
    "}",
  ],
  taskCancel: [
    "function $taskCancel(task) {",
    "  if (task.outcome === undefined) task.cancelled = true;",
    "  return undefined;",
    "}",
  ],
  taskCanceled: [
    "function $taskCanceled(task) {",
    "  return task.cancelled;",
    "}",
  ],
  channel: [
    "function $channel() {",
    "  return { $channel: true, capacity: 16, buffer: [], closed: false };",
    "}",
    "function $channelError(message) {",
    '  return { $type: "ChannelError", 0: message };',
    "}",
    "function $channelSend(channel, value) {",
    '  if (channel.closed) return $err($channelError("send on a closed channel"));',
    "  if (channel.buffer.length >= channel.capacity) {",
    '    $panic("channel deadlock: send on a full bounded channel would block (single-threaded runtime)");',
    "  }",
    "  channel.buffer.push(value);",
    "  return $ok(undefined);",
    "}",
    "function $channelReceive(channel) {",
    "  if (channel.buffer.length === 0) {",
    '    if (channel.closed) return $err($channelError("receive on a closed channel"));',
    '    $panic("channel deadlock: receive on an empty channel would block (single-threaded runtime)");',
    "  }",
    "  return $ok(channel.buffer.shift());",
    "}",
    "function $channelClose(channel) {",
    "  channel.closed = true;",
    "  return undefined;",
    "}",
  ],
  renderFloat: [
    "function $renderFloat(value) {",
    '  if (Number.isNaN(value)) return "NaN";',
    '  if (value === Infinity) return "Infinity";',
    '  if (value === -Infinity) return "-Infinity";',
    "  if (Number.isInteger(value) && Math.abs(value) < 1e16) return value.toFixed(1);",
    "  return String(value);",
    "}",
  ],
  render: [
    "function $render(value, debug = false) {",
    '  if (value === undefined) return "()";',
    '  if (value === null) return "None";',
    '  if (typeof value === "bigint") return value.toString();',
    '  if (typeof value === "string") return debug ? JSON.stringify(value) : value;',
    '  if (typeof value === "number" || typeof value === "boolean") return String(value);',
    '  if (value instanceof Uint8Array) {',
    '    let output = "b\\"";',
    "    for (const byte of value) {",
    '      if (byte === 92) output += "\\\\\\\\";',
    '      else if (byte === 34) output += "\\\\\\"";',
    '      else if (byte === 10) output += "\\\\n";',
    '      else if (byte === 13) output += "\\\\r";',
    '      else if (byte === 9) output += "\\\\t";',
    '      else if (byte >= 32 && byte <= 126) output += String.fromCharCode(byte);',
    '      else output += "\\\\u{" + byte.toString(16).padStart(2, "0") + "}";',
    "    }",
    '    return output + "\\"";',
    "  }",
    "  if (Array.isArray(value)) {",
    '    return "[" + value.map((item) => $render(item, debug)).join(", ") + "]";',
    "  }",
    "  if (value.$tuple) {",
    '    const items = value.$tuple.map((item) => $render(item, debug));',
    '    return "(" + items.join(", ") + (items.length === 1 ? "," : "") + ")";',
    "  }",
    "  if (value.$map) {",
    '    return "{" + value.$map.map(([key, item]) => $render(key, debug) + ": " + $render(item, debug)).join(", ") + "}";',
    "  }",
    "  if (typeof value.$float === \"number\") return $renderFloat(value.$float);",
    "  if (typeof value.$option === \"boolean\") {",
    '    return value.$option ? "Some(" + $render(value.value, debug) + ")" : "None";',
    "  }",
    "  if (value.$result) {",
    '    return value.ok ? "Ok(" + $render(value.value, debug) + ")" : "Err(" + $render(value.error, debug) + ")";',
    "  }",
    "  if (value.$tag) {",
    '    let output = value.$enum + "." + value.$tag;',
    '    const fields = Object.keys(value).filter((key) => /^[0-9]+$/.test(key)).sort((a, b) => Number(a) - Number(b));',
    "    if (fields.length > 0) {",
    '      output += "(" + fields.map((key) => $render(value[key], debug)).join(", ") + ")";',
    "    }",
    "    return output;",
    "  }",
    "  if (value.$type) {",
    "    const numeric = Object.keys(value).filter((key) => /^[0-9]+$/.test(key));",
    "    if (numeric.length > 0) {",
    '      const fields = numeric.sort((a, b) => Number(a) - Number(b)).map((key) => $render(value[key], debug));',
    '      return value.$type + "(" + fields.join(", ") + ")";',
    "    }",
    '    const fields = Object.keys(value).filter((key) => key !== "$type").map((key) => key + ": " + $render(value[key], debug));',
    '    return value.$type + " { " + fields.join(", ") + " }";',
    "  }",
    "  return String(value);",
    "}",
  ],
  renderDebug: [
    "function $renderDebug(value) {",
    "  return $render(value, true);",
    "}",
  ],
  float: [
    "function $float(value) {",
    "  return {",
    "    $float: Number(value),",
    "    valueOf() { return this.$float; },",
    "  };",
    "}",
  ],
  write: [
    "function $write(text) {",
    '  if (typeof process !== "undefined" && process.stdout && typeof process.stdout.write === "function") {',
    "    process.stdout.write(text);",
    "    return;",
    "  }",
    '  if (typeof console !== "undefined") console.log(text);',
    "}",
  ],
  print: [
    "function $print(value) {",
    "  $write($render(value, false));",
    "}",
  ],
  println: [
    "function $println(value) {",
    '  $write($render(value, false) + "\\n");',
    "}",
  ],
  dbg: [
    "function $dbg(value) {",
    "  const line = $render(value, true);",
    '  if (typeof process !== "undefined" && process.stderr && typeof process.stderr.write === "function") {',
    '    process.stderr.write(line + "\\n");',
    "    return;",
    "  }",
    '  if (typeof console !== "undefined") console.error(line);',
    "}",
  ],
  assert: [
    "function $assert(condition) {",
    '  if (!condition) $panic("assertion failed");',
    "}",
  ],
  utf8Len: [
    "function $utf8Len(value) {",
    "  return new TextEncoder().encode(value).length;",
    "}",
  ],
  len: [
    "function $len(value) {",
    '  if (typeof value === "string") return $utf8Len(value);',
    "  if (Array.isArray(value) || value instanceof Uint8Array) return value.length;",
    "  if (value && value.$map) return value.$map.length;",
    '  $panic("len() is not implemented for this value");',
    "}",
  ],
  lenBig: [
    "function $lenBig(value) {",
    '  if (typeof value === "string") return BigInt(new TextEncoder().encode(value).length);',
    "  if (Array.isArray(value) || value instanceof Uint8Array) return BigInt(value.length);",
    "  if (value && value.$map) return BigInt(value.$map.length);",
    '  $panic("len() is not implemented for this value");',
    "}",
  ],
  strAt: [
    "function $strAt(value, index) {",
    "  const characters = Array.from(value);",
    "  const position = Number(index);",
    '  if (!Number.isInteger(position) || position < 0 || position >= characters.length) $panic("index out of bounds");',
    "  return characters[position];",
    "}",
  ],
  charCount: [
    "function $charCount(value) {",
    "  return Array.from(value).length;",
    "}",
  ],
  strBytes: [
    "function $strBytes(value) {",
    "  return Array.from(new TextEncoder().encode(value));",
    "}",
  ],
  strChars: [
    "function $strChars(value) {",
    "  return Array.from(value);",
    "}",
  ],
  byteAt: [
    "function $byteAt(value, index) {",
    "  const bytes = new TextEncoder().encode(value);",
    "  const position = Number(index);",
    '  if (!Number.isInteger(position) || position < 0 || position >= bytes.length) return $none;',
    "  return $some(bytes[position]);",
    "}",
  ],
  charAt: [
    "function $charAt(value, index) {",
    "  const characters = Array.from(value);",
    "  const position = Number(index);",
    '  if (!Number.isInteger(position) || position < 0 || position >= characters.length) return $none;',
    "  return $some(characters[position]);",
    "}",
  ],
  strSlice: [
    "function $strSlice(value, start, end) {",
    "  const bytes = new TextEncoder().encode(value);",
    "  const from = Number(start);",
    "  const to = Number(end);",
    "  const boundary = (position) =>",
    "    position === bytes.length || (position >= 0 && position < bytes.length && (bytes[position] & 0xc0) !== 0x80);",
    "  if (!Number.isInteger(from) || !Number.isInteger(to) || from < 0 || to < from || to > bytes.length || !boundary(from) || !boundary(to)) {",
    '    return $err({ $type: "SliceError", 0: "slice range is out of bounds or splits a UTF-8 scalar value" });',
    "  }",
    '  return $ok(new TextDecoder("utf-8").decode(bytes.subarray(from, to)));',
    "}",
  ],
  bytesNew: [
    "function $bytesNew(length) {",
    "  return new Uint8Array(Number(length));",
    "}",
  ],
  byteSlice: [
    "function $byteSlice(value, start, end) {",
    "  const from = Number(start);",
    "  const to = Number(end);",
    "  if (!Number.isInteger(from) || !Number.isInteger(to) || from < 0 || to < from || to > value.length) $panic(`bytes slice ${from}..${to} is out of bounds`);",
    "  return value.slice(from, to);",
    "}",
  ],
  index: [
    "function $index(value, index) {",
    "  const position = Number(index);",
    '  if (!Number.isInteger(position) || position < 0 || position >= value.length) $panic("index out of bounds");',
    "  return value[position];",
    "}",
  ],
  indexSet: [
    "function $indexSet(value, index, item) {",
    "  const position = Number(index);",
    '  if (!Number.isInteger(position) || position < 0 || position >= value.length) $panic("index out of bounds");',
    "  value[position] = item;",
    "  return item;",
    "}",
  ],
  getArray: [
    "function $getArray(value, index) {",
    "  const position = Number(index);",
    '  if (!Number.isInteger(position) || position < 0 || position >= value.length) return $none;',
    "  return $some(value[position]);",
    "}",
  ],
  tuple: [
    "function $tuple(values) {",
    "  return { $tuple: values };",
    "}",
  ],
  tupleAt: [
    "function $tupleAt(value, index) {",
    "  const position = Number(index);",
    '  if (!Number.isInteger(position) || position < 0 || position >= value.$tuple.length) $panic("index out of bounds");',
    "  return value.$tuple[position];",
    "}",
  ],
  map: [
    "function $map(entries) {",
    "  const values = [];",
    "  for (const [key, value] of entries) {",
    "    const existing = values.findIndex(([candidate]) => $eq(candidate, key));",
    "    if (existing >= 0) values[existing] = [key, value];",
    "    else values.push([key, value]);",
    "  }",
    "  return { $map: values };",
    "}",
  ],
  mapGet: [
    "function $mapGet(map, key) {",
    "  const entry = map.$map.find(([candidate]) => $eq(candidate, key));",
    "  return entry === undefined ? $none : $some(entry[1]);",
    "}",
  ],
  mapInsert: [
    "function $mapInsert(map, key, value) {",
    "  const index = map.$map.findIndex(([candidate]) => $eq(candidate, key));",
    "  if (index < 0) {",
    "    map.$map.push([key, value]);",
    "    return $none;",
    "  }",
    "  const previous = map.$map[index][1];",
    "  map.$map[index] = [key, value];",
    "  return $some(previous);",
    "}",
  ],
  mapIndex: [
    "function $mapIndex(map, key) {",
    "  const entry = map.$map.find(([candidate]) => $eq(candidate, key));",
    '  if (entry === undefined) $panic("map key is not present");',
    "  return entry[1];",
    "}",
  ],
  mapIndexSet: [
    "function $mapIndexSet(map, key, value) {",
    "  const index = map.$map.findIndex(([candidate]) => $eq(candidate, key));",
    "  if (index < 0) map.$map.push([key, value]);",
    "  else map.$map[index] = [key, value];",
    "  return undefined;",
    "}",
  ],
  mapKeys: [
    "function $mapKeys(map) {",
    "  return map.$map.map(([key]) => key);",
    "}",
  ],
  eq: [
    "function $eq(left, right) {",
    "  if (left === right) return true;",
    '  if (typeof left !== "object" || typeof right !== "object" || left === null || right === null) return false;',
    "  if (typeof left.$float === \"number\" || typeof right.$float === \"number\") {",
    "    return typeof left.$float === \"number\" && typeof right.$float === \"number\" && left.$float === right.$float;",
    "  }",
    "  if (left instanceof Uint8Array || right instanceof Uint8Array) {",
    "    if (!(left instanceof Uint8Array) || !(right instanceof Uint8Array)) return false;",
    "    if (left.length !== right.length) return false;",
    "    for (let index = 0; index < left.length; index += 1) if (left[index] !== right[index]) return false;",
    "    return true;",
    "  }",
    "  if (Array.isArray(left) || Array.isArray(right)) {",
    "    if (!Array.isArray(left) || !Array.isArray(right)) return false;",
    "    return left.length === right.length && left.every((item, index) => $eq(item, right[index]));",
    "  }",
    "  if (left.$tuple || right.$tuple) {",
    "    return Boolean(left.$tuple && right.$tuple) && $eq(left.$tuple, right.$tuple);",
    "  }",
    "  if (left.$map || right.$map) {",
    "    if (!left.$map || !right.$map || left.$map.length !== right.$map.length) return false;",
    "    return left.$map.every(([key, value]) => {",
    "      const entry = right.$map.find(([candidate]) => $eq(candidate, key));",
    "      return entry !== undefined && $eq(value, entry[1]);",
    "    });",
    "  }",
    "  if (typeof left.$option === \"boolean\" || typeof right.$option === \"boolean\") {",
    "    return typeof left.$option === \"boolean\" && typeof right.$option === \"boolean\" &&",
    "      left.$option === right.$option && (left.$option === false || $eq(left.value, right.value));",
    "  }",
    "  if (left.$result || right.$result) {",
    "    return Boolean(left.$result && right.$result) && left.ok === right.ok &&",
    "      (left.ok ? $eq(left.value, right.value) : $eq(left.error, right.error));",
    "  }",
    "  if (left.$tag || right.$tag) {",
    "    if (!left.$tag || !right.$tag || left.$enum !== right.$enum || left.$tag !== right.$tag) return false;",
    "    const keys = Object.keys(left).filter((key) => /^[0-9]+$/.test(key));",
    "    if (keys.length !== Object.keys(right).filter((key) => /^[0-9]+$/.test(key)).length) return false;",
    "    return keys.every((key) => $eq(left[key], right[key]));",
    "  }",
    "  if (left.$type || right.$type) {",
    "    if (!left.$type || !right.$type || left.$type !== right.$type) return false;",
    "    const keys = Object.keys(left).filter((key) => key !== \"$type\");",
    "    if (keys.length !== Object.keys(right).filter((key) => key !== \"$type\").length) return false;",
    "    return keys.every((key) => Object.prototype.hasOwnProperty.call(right, key) && $eq(left[key], right[key]));",
    "  }",
    "  return false;",
    "}",
  ],
  none: ["const $none = Object.freeze({ $option: false });"],
  some: [
    "function $some(value) {",
    "  return { $option: true, value };",
    "}",
  ],
  ok: [
    "function $ok(value) {",
    "  return { $result: true, ok: true, value };",
    "}",
  ],
  err: [
    "function $err(error) {",
    "  return { $result: true, ok: false, error };",
    "}",
  ],
  unwrapOption: [
    "function $unwrapOption(value) {",
    '  if (value.$option === false) $panic("called unwrap() on None");',
    "  return value.value;",
    "}",
  ],
  unwrapResult: [
    "function $unwrapResult(value) {",
    '  if (!value.ok) $panic("called unwrap() on Err(" + $render(value.error) + ")");',
    "  return value.value;",
    "}",
  ],
  resultError: [
    "function $resultError(value) {",
    '  if (value.ok) $panic("called error() on Ok(" + $render(value.value) + ")");',
    "  return value.error;",
    "}",
  ],
  parseInt: [
    "function $parseInt(value) {",
    '  if (!/^[+-]?[0-9]+$/.test(value)) return $err({ $type: "ParseIntError", 0: value });',
    "  const parsed = Number(value);",
    "  if (!Number.isInteger(parsed) || parsed < -2147483648 || parsed > 2147483647) {",
    '    return $err({ $type: "ParseIntError", 0: value });',
    "  }",
    "  return $ok(parsed);",
    "}",
  ],
  parseIntBig: [
    "function $parseIntBig(value) {",
    '  if (!/^[+-]?[0-9]+$/.test(value)) return $err({ $type: "ParseIntError", 0: value });',
    "  const parsed = BigInt(value);",
    "  if (parsed < -2147483648n || parsed > 2147483647n) {",
    '    return $err({ $type: "ParseIntError", 0: value });',
    "  }",
    "  return $ok(parsed);",
    "}",
  ],
  div: [
    "function $div(left, right) {",
    '  if (right === 0) $panic("division by zero");',
    "  return Math.trunc(left / right);",
    "}",
  ],
  rem: [
    "function $rem(left, right) {",
    '  if (right === 0) $panic("division by zero");',
    "  return left % right;",
    "}",
  ],
  divBig: [
    "function $divBig(left, right) {",
    "  if (right === 0n) $panic(\"division by zero\");",
    "  return left / right;",
    "}",
  ],
  remBig: [
    "function $remBig(left, right) {",
    "  if (right === 0n) $panic(\"division by zero\");",
    "  return left % right;",
    "}",
  ],
  i8: ["function $i8(value) {", "  return (value << 24) >> 24;", "}"],
  i16: ["function $i16(value) {", "  return (value << 16) >> 16;", "}"],
  i32: ["function $i32(value) {", "  return value | 0;", "}"],
  u8: ["function $u8(value) {", "  return value & 0xff;", "}"],
  u16: ["function $u16(value) {", "  return value & 0xffff;", "}"],
  u32: ["function $u32(value) {", "  return value >>> 0;", "}"],
  i64: ["function $i64(value) {", "  return BigInt.asIntN(64, BigInt(value));", "}"],
  u64: ["function $u64(value) {", "  return BigInt.asUintN(64, BigInt(value));", "}"],
  checked_i8: [
    "function $checked_i8(value) {",
    '  if (value < -128 || value > 127) $panic("integer overflow");',
    "  return value;",
    "}",
  ],
  checked_i16: [
    "function $checked_i16(value) {",
    '  if (value < -32768 || value > 32767) $panic("integer overflow");',
    "  return value;",
    "}",
  ],
  checked_i32: [
    "function $checked_i32(value) {",
    '  if (value < -2147483648 || value > 2147483647) $panic("integer overflow");',
    "  return value;",
    "}",
  ],
  checked_u8: [
    "function $checked_u8(value) {",
    '  if (value < 0 || value > 255) $panic("integer overflow");',
    "  return value;",
    "}",
  ],
  checked_u16: [
    "function $checked_u16(value) {",
    '  if (value < 0 || value > 65535) $panic("integer overflow");',
    "  return value;",
    "}",
  ],
  checked_u32: [
    "function $checked_u32(value) {",
    '  if (value < 0 || value > 4294967295) $panic("integer overflow");',
    "  return value;",
    "}",
  ],
  checked_i64: [
    "function $checked_i64(value) {",
    '  if (value < -9223372036854775808n || value > 9223372036854775807n) $panic("integer overflow");',
    "  return value;",
    "}",
  ],
  checked_u64: [
    "function $checked_u64(value) {",
    '  if (value < 0n || value > 18446744073709551615n) $panic("integer overflow");',
    "  return value;",
    "}",
  ],
  checkedInteger: [
    "function $checkedInteger(tag, operation, left, right) {",
    "  const bounds = {",
    "    i8: [-128, 127],",
    "    i16: [-32768, 32767],",
    "    i32: [-2147483648, 2147483647],",
    "    i64: [-9223372036854775808n, 9223372036854775807n],",
    "    u8: [0, 255],",
    "    u16: [0, 65535],",
    "    u32: [0, 4294967295],",
    "    u64: [0n, 18446744073709551615n],",
    "  };",
    "  const [min, max] = bounds[tag];",
    "  let value;",
    "  if (operation === \"div\") {",
    "    if (right === 0 || right === 0n) $panic(\"division by zero\");",
    '    value = typeof left === "bigint" ? left / right : Math.trunc(left / right);',
    '  } else if (operation === "add") {',
    "    value = left + right;",
    '  } else if (operation === "sub") {',
    "    value = left - right;",
    '  } else if (operation === "mul") {',
    "    value = left * right;",
    "  } else {",
    '    $panic("unsupported checked integer operation");',
    "  }",
    "  if (value < min || value > max) {",
    '    return $err({ $type: "OverflowError", 0: "integer overflow in \'checked_" + operation + "\'" });',
    "  }",
    "  return $ok(value);",
    "}",
  ],
};

type LooseTypeName =
  | "$Dyn"
  | "AstraTask"
  | "AstraChannel"
  | "ParseIntError"
  | "SliceError"
  | "TaskError"
  | "ChannelError"
  | "OverflowError"
  | "ScriptError";

const BUILTIN_ERROR_TYPES: readonly LooseTypeName[] = [
  "ParseIntError",
  "SliceError",
  "TaskError",
  "ChannelError",
  "OverflowError",
  "ScriptError",
];

const LOOSE_TYPE_DECLARATIONS: Record<LooseTypeName, string> = {
  $Dyn: [
    "type $Dyn = {",
    "  readonly $failure?: boolean;",
    "  readonly $result?: boolean;",
    "  readonly ok?: boolean;",
    "  readonly value?: unknown;",
    "  readonly error?: unknown;",
    "  readonly $option?: boolean;",
    "  readonly $tuple?: readonly unknown[];",
    "  readonly $map?: Array<readonly [unknown, unknown]>;",
    "  readonly $float?: number;",
    "  readonly $tag?: string;",
    "  readonly $enum?: string;",
    "  readonly $type?: string;",
    "  readonly length?: number;",
    "  readonly [index: number]: unknown;",
    "};",
  ].join("\n"),
  AstraTask: [
    "interface AstraTask<T> {",
    "  readonly $task: true;",
    "  cancelled: boolean;",
    "  joined: boolean;",
    "  outcome:",
    "    | { readonly ok: true; readonly value: T }",
    "    | { readonly ok: false; readonly error: unknown }",
    "    | { readonly panic: unknown }",
    "    | undefined;",
    "  join(): T;",
    "  cancel(): undefined;",
    "  canceled(): boolean;",
    "}",
  ].join("\n"),
  AstraChannel: [
    "interface AstraChannel<T> {",
    "  readonly $channel: true;",
    "  capacity: number;",
    "  buffer: T[];",
    "  closed: boolean;",
    "}",
  ].join("\n"),
  ParseIntError: 'interface ParseIntError { readonly $type: "ParseIntError"; readonly 0: string }',
  SliceError: 'interface SliceError { readonly $type: "SliceError"; readonly 0: string }',
  TaskError: 'interface TaskError { readonly $type: "TaskError"; readonly 0: string }',
  ChannelError: 'interface ChannelError { readonly $type: "ChannelError"; readonly 0: string }',
  OverflowError: 'interface OverflowError { readonly $type: "OverflowError"; readonly 0: string }',
  ScriptError: 'interface ScriptError { readonly $type: "ScriptError"; readonly 0: string }',
};

const LOOSE_TYPE_ORDER: readonly LooseTypeName[] = [
  "$Dyn",
  "AstraTask",
  "AstraChannel",
  "ParseIntError",
  "SliceError",
  "TaskError",
  "ChannelError",
  "OverflowError",
  "ScriptError",
];

const INTEGER_TAGS = new Set(["I8", "I16", "I32", "I64", "U8", "U16", "U32", "U64"]);

const TS_HELPER_SIGNATURES: Partial<Record<HelperName, string>> = {
  panic: "function $panic(message: string): never {",
  abort: "function $abort(message: string): never {",
  failure: "function $failure(error: unknown): { readonly $failure: true; readonly error: unknown } {",
  fail: "function $fail(error: unknown): never {",
  taskError: "function $taskError(value: unknown): never {",
  spawn: "function $spawn<T>(body: () => T): AstraTask<T> {",
  taskJoin: "function $taskJoin<T>(task: AstraTask<T>): T | { readonly $failure: true; readonly error: unknown } {",
  taskCancel: "function $taskCancel<T>(task: AstraTask<T>): undefined {",
  taskCanceled: "function $taskCanceled<T>(task: AstraTask<T>): boolean {",
  channel: "function $channel<T>(): AstraChannel<T> {",
  renderFloat: "function $renderFloat(value: number): string {",
  render: "function $render(value: unknown, debug = false): string {",
  renderDebug: "function $renderDebug(value: unknown): string {",
  float: "function $float(value: number): number {",
  write: "function $write(text: string): undefined {",
  print: "function $print(value: unknown): undefined {",
  println: "function $println(value: unknown): undefined {",
  dbg: "function $dbg(value: unknown): undefined {",
  assert: "function $assert(condition: boolean): undefined {",
  utf8Len: "function $utf8Len(value: string): number {",
  len: "function $len(value: unknown): number {",
  lenBig: "function $lenBig(value: unknown): bigint {",
  strAt: "function $strAt(value: string, index: number | bigint): string {",
  charCount: "function $charCount(value: string): number {",
  strBytes: "function $strBytes(value: string): number[] {",
  strChars: "function $strChars(value: string): string[] {",
  byteAt: "function $byteAt(value: string, index: number | bigint): { readonly $option: true; readonly value: number } | { readonly $option: false } {",
  charAt: "function $charAt(value: string, index: number | bigint): { readonly $option: true; readonly value: string } | { readonly $option: false } {",
  strSlice: "function $strSlice(value: string, start: number | bigint, end: number | bigint): { readonly $result: true; readonly ok: true; readonly value: string } | { readonly $result: true; readonly ok: false; readonly error: SliceError } {",
  bytesNew: "function $bytesNew(length: number | bigint): Uint8Array {",
  byteSlice: "function $byteSlice(value: Uint8Array, start: number | bigint, end: number | bigint): Uint8Array {",
  index: "function $index<T>(value: readonly T[] | ArrayLike<T>, index: number | bigint): T {",
  indexSet: "function $indexSet<T>(value: T[] | { length: number; [n: number]: T }, index: number | bigint, item: T): T {",
  getArray: "function $getArray<T>(value: readonly T[], index: number | bigint): { readonly $option: true; readonly value: T } | { readonly $option: false } {",
  tuple: "function $tuple<T extends readonly unknown[]>(values: T): { readonly $tuple: T } {",
  tupleAt: "function $tupleAt<T extends readonly unknown[], I extends number>(value: { readonly $tuple: T }, index: I | number | bigint): T[I] {",
  map: "function $map<K, V>(entries: ReadonlyArray<readonly [K, V]>): { readonly $map: Array<readonly [K, V]> } {",
  mapGet: "function $mapGet<K, V>(map: { readonly $map: ReadonlyArray<readonly [K, V]> }, key: K): { readonly $option: true; readonly value: V } | { readonly $option: false } {",
  mapInsert: "function $mapInsert<K, V>(map: { readonly $map: Array<readonly [K, V]> }, key: K, value: V): { readonly $option: true; readonly value: V } | { readonly $option: false } {",
  mapKeys: "function $mapKeys<K, V>(map: { readonly $map: ReadonlyArray<readonly [K, V]> }): K[] {",
  eq: "function $eq(left: unknown, right: unknown): boolean {",
  mapIndex: "function $mapIndex<K, V>(map: { readonly $map: ReadonlyArray<readonly [K, V]> }, key: K): V {",
  mapIndexSet: "function $mapIndexSet<K, V>(map: { readonly $map: Array<readonly [K, V]> }, key: K, value: V): undefined {",
  none: "const $none: { readonly $option: false } = Object.freeze({ $option: false });",
  some: "function $some<T>(value: T): { readonly $option: true; readonly value: T } {",
  ok: "function $ok<T>(value: T): { readonly $result: true; readonly ok: true; readonly value: T } {",
  err: "function $err<E>(error: E): { readonly $result: true; readonly ok: false; readonly error: E } {",
  unwrapOption: "function $unwrapOption<T>(value: { readonly $option: true; readonly value: T } | { readonly $option: false }): T {",
  unwrapResult: "function $unwrapResult<T, E>(value: { readonly $result: true; readonly ok: true; readonly value: T } | { readonly $result: true; readonly ok: false; readonly error: E }): T {",
  resultError: "function $resultError<T, E>(value: { readonly $result: true; readonly ok: true; readonly value: T } | { readonly $result: true; readonly ok: false; readonly error: E }): E {",
  parseInt: "function $parseInt(value: string): { readonly $result: true; readonly ok: true; readonly value: number } | { readonly $result: true; readonly ok: false; readonly error: ParseIntError } {",
  parseIntBig: "function $parseIntBig(value: string): { readonly $result: true; readonly ok: true; readonly value: bigint } | { readonly $result: true; readonly ok: false; readonly error: ParseIntError } {",
  div: "function $div(left: number, right: number): number {",
  rem: "function $rem(left: number, right: number): number {",
  divBig: "function $divBig(left: bigint, right: bigint): bigint {",
  remBig: "function $remBig(left: bigint, right: bigint): bigint {",
  i8: "function $i8(value: number): number {",
  i16: "function $i16(value: number): number {",
  i32: "function $i32(value: number): number {",
  u8: "function $u8(value: number): number {",
  u16: "function $u16(value: number): number {",
  u32: "function $u32(value: number): number {",
  i64: "function $i64(value: number | bigint): bigint {",
  u64: "function $u64(value: number | bigint): bigint {",
  checked_i8: "function $checked_i8(value: number): number {",
  checked_i16: "function $checked_i16(value: number): number {",
  checked_i32: "function $checked_i32(value: number): number {",
  checked_u8: "function $checked_u8(value: number): number {",
  checked_u16: "function $checked_u16(value: number): number {",
  checked_u32: "function $checked_u32(value: number): number {",
  checked_i64: "function $checked_i64(value: bigint): bigint {",
  checked_u64: "function $checked_u64(value: bigint): bigint {",
  checkedInteger:
    "function $checkedInteger(tag: string, operation: string, left: number | bigint, right: number | bigint): { readonly $result: true; readonly ok: true; readonly value: number | bigint } | { readonly $result: true; readonly ok: false; readonly error: OverflowError } {",
};

const TS_HELPER_OVERRIDES: Partial<Record<HelperName, readonly string[]>> = {
  abort: [
    "function $abort(message: string): never {",
    "  const runtime = globalThis as unknown as {",
    "    readonly process?: {",
    "      readonly exit?: (code: number) => never;",
    "      readonly stderr?: { readonly write?: (chunk: string) => void };",
    "    };",
    "    readonly close?: () => void;",
    "  };",
    "  if (runtime.process !== undefined && typeof runtime.process.exit === \"function\") {",
    "    if (runtime.process.stderr && typeof runtime.process.stderr.write === \"function\") {",
    "      runtime.process.stderr.write(\"abort: \" + message + \"\\n\");",
    "    }",
    "    runtime.process.exit(134);",
    "  }",
    "  if (typeof runtime.close === \"function\") runtime.close();",
    "  throw new Error(\"abort: \" + message);",
    "}",
  ],
  write: [
    "function $write(text: string): undefined {",
    "  const runtime = globalThis as unknown as {",
    "    readonly process?: { readonly stdout?: { readonly write?: (chunk: string) => void } };",
    "    readonly console?: { readonly log?: (message: unknown) => void };",
    "  };",
    '  if (runtime.process !== undefined && runtime.process.stdout && typeof runtime.process.stdout.write === "function") {',
    "    runtime.process.stdout.write(text);",
    "    return undefined;",
    "  }",
    "  if (runtime.console !== undefined) (runtime.console.log as (message: unknown) => void)(text);",
    "  return undefined;",
    "}",
  ],
  dbg: [
    "function $dbg(value: unknown): undefined {",
    "  const line = $render(value, true);",
    "  const runtime = globalThis as unknown as {",
    "    readonly process?: { readonly stderr?: { readonly write?: (chunk: string) => void } };",
    "    readonly console?: { readonly error?: (message: unknown) => void };",
    "  };",
    '  if (runtime.process !== undefined && runtime.process.stderr && typeof runtime.process.stderr.write === "function") {',
    '    runtime.process.stderr.write(line + "\\n");',
    "    return undefined;",
    "  }",
    "  if (runtime.console !== undefined) (runtime.console.error as (message: unknown) => void)(line);",
    "  return undefined;",
    "}",
  ],
  map: [
    "function $map<K, V>(entries: ReadonlyArray<readonly [K, V]>): { readonly $map: Array<readonly [K, V]> } {",
    "  const values: Array<readonly [K, V]> = [];",
    "  for (const [key, value] of entries) {",
    "    const existing = values.findIndex(([candidate]) => $eq(candidate, key));",
    "    if (existing >= 0) values[existing] = [key, value];",
    "    else values.push([key, value]);",
    "  }",
    "  return { $map: values };",
    "}",
  ],
  float: [
    "function $float(value: number): number {",
    "  const boxed: { readonly $float: number; valueOf(): number } = {",
    "    $float: Number(value),",
    "    valueOf(): number { return this.$float; },",
    "  };",
    "  return boxed as unknown as number;",
    "}",
  ],
  taskScope: [
    "const $taskScopes: Array<{ tasks: Array<AstraTask<unknown>> }> = [];",
    "function $enterTaskScope(): { tasks: Array<AstraTask<unknown>> } {",
    "  const scope: { tasks: Array<AstraTask<unknown>> } = { tasks: [] };",
    "  $taskScopes.push(scope);",
    "  return scope;",
    "}",
    "function $exitTaskScope(scope: { tasks: Array<AstraTask<unknown>> }): undefined {",
    "  $taskScopes.pop();",
    "  for (const task of scope.tasks) {",
    "    if (task.joined) continue;",
    "    const outcome = task.outcome as { readonly ok?: boolean; readonly panic?: unknown } | undefined;",
    "    if (outcome === undefined) {",
    '      if (task.cancelled) $taskError("task was cancelled");',
    '      $taskError("task failed without a value");',
    "    }",
    "    if (outcome.panic !== undefined) $taskError(outcome.panic);",
    "  }",
    "  return undefined;",
    "}",
  ],
  spawn: [
    "function $spawn<T>(body: () => T): AstraTask<T> {",
    "  const task: AstraTask<T> = {",
    "    $task: true,",
    "    cancelled: false,",
    "    joined: false,",
    "    outcome: undefined,",
    "    join() { return $taskJoin(this) as T; },",
    "    cancel() { $taskCancel(this); },",
    "    canceled() { return $taskCanceled(this); },",
    "  };",
    "  try {",
    "    task.outcome = { ok: true, value: body() };",
    "  } catch (error) {",
    '    const failure = error as { $failure?: boolean; error?: unknown } | null | undefined;',
    "    if (failure !== null && failure !== undefined && failure.$failure) task.outcome = { ok: false, error: failure.error };",
    "    else task.outcome = { panic: error };",
    "  }",
    "  if ($taskScopes.length > 0) $taskScopes[$taskScopes.length - 1].tasks.push(task);",
    "  return task;",
    "}",
  ],
  taskJoin: [
    "function $taskJoin<T>(task: AstraTask<T>): T | { readonly $failure: true; readonly error: unknown } {",
    "  task.joined = true;",
    "  const outcome = task.outcome as",
    "    | { readonly ok?: boolean; readonly error?: unknown; readonly panic?: unknown }",
    "    | undefined;",
    "  if (outcome === undefined) {",
    '    if (task.cancelled) $taskError("task was cancelled");',
    '    $taskError("task failed without a value");',
    "  }",
    "  if (outcome.panic !== undefined) $taskError(outcome.panic);",
    "  if (!outcome.ok) return $failure(outcome.error);",
    "  return (task.outcome as { readonly ok: true; readonly value: T }).value;",
    "}",
  ],
  channel: [
    "function $channel<T>(): AstraChannel<T> {",
    "  return { $channel: true, capacity: 16, buffer: [], closed: false };",
    "}",
    "function $channelError(message: string): ChannelError {",
    '  return { $type: "ChannelError", 0: message };',
    "}",
    "function $channelSend<T>(channel: AstraChannel<T>, value: T): { readonly $result: true; readonly ok: true; readonly value: undefined } | { readonly $result: true; readonly ok: false; readonly error: ChannelError } {",
    '  if (channel.closed) return $err($channelError("send on a closed channel"));',
    "  if (channel.buffer.length >= channel.capacity) {",
    '    $panic("channel deadlock: send on a full bounded channel would block (single-threaded runtime)");',
    "  }",
    "  channel.buffer.push(value);",
    "  return $ok(undefined);",
    "}",
    "function $channelReceive<T>(channel: AstraChannel<T>): { readonly $result: true; readonly ok: true; readonly value: T } | { readonly $result: true; readonly ok: false; readonly error: ChannelError } {",
    "  if (channel.buffer.length === 0) {",
    '    if (channel.closed) return $err($channelError("receive on a closed channel"));',
    '    $panic("channel deadlock: receive on an empty channel would block (single-threaded runtime)");',
    "  }",
    "  return $ok(channel.buffer.shift() as T);",
    "}",
    "function $channelClose<T>(channel: AstraChannel<T>): undefined {",
    "  channel.closed = true;",
    "  return undefined;",
    "}",
  ],
  render: [
    "function $render(value: unknown, debug = false): string {",
    "  const input = value as $Dyn;",
    '  if (value === undefined) return "()";',
    '  if (value === null) return "None";',
    '  if (typeof value === "bigint") return value.toString();',
    '  if (typeof value === "string") return debug ? JSON.stringify(value) : value;',
    '  if (typeof value === "number" || typeof value === "boolean") return String(value);',
    "  if (value instanceof Uint8Array) {",
    '    let output = "b\\"";',
    "    for (const byte of value) {",
    '      if (byte === 92) output += "\\\\\\\\";',
    '      else if (byte === 34) output += "\\\\\\"";',
    '      else if (byte === 10) output += "\\\\n";',
    '      else if (byte === 13) output += "\\\\r";',
    '      else if (byte === 9) output += "\\\\t";',
    '      else if (byte >= 32 && byte <= 126) output += String.fromCharCode(byte);',
    '      else output += "\\\\u{" + byte.toString(16).padStart(2, "0") + "}";',
    "    }",
    '    return output + "\\"";',
    "  }",
    "  if (Array.isArray(value)) {",
    '    return "[" + value.map((item) => $render(item, debug)).join(", ") + "]";',
    "  }",
    "  if (input.$tuple) {",
    '    const items = input.$tuple.map((item) => $render(item, debug));',
    '    return "(" + items.join(", ") + (items.length === 1 ? "," : "") + ")";',
    "  }",
    "  if (input.$map) {",
    '    return "{" + input.$map.map(([key, item]) => $render(key, debug) + ": " + $render(item, debug)).join(", ") + "}";',
    "  }",
    '  if (typeof input.$float === "number") return $renderFloat(input.$float);',
    '  if (typeof input.$option === "boolean") {',
    '    return input.$option ? "Some(" + $render(input.value, debug) + ")" : "None";',
    "  }",
    "  if (input.$result) {",
    '    return input.ok ? "Ok(" + $render(input.value, debug) + ")" : "Err(" + $render(input.error, debug) + ")";',
    "  }",
    "  if (input.$tag) {",
    '    let output = input.$enum + "." + input.$tag;',
    '    const fields = Object.keys(input).filter((key) => /^[0-9]+$/.test(key)).sort((a, b) => Number(a) - Number(b));',
    "    if (fields.length > 0) {",
    '      output += "(" + fields.map((key) => $render(input[Number(key)], debug)).join(", ") + ")";',
    "    }",
    "    return output;",
    "  }",
    "  if (input.$type) {",
    '    const numeric = Object.keys(input).filter((key) => /^[0-9]+$/.test(key));',
    "    if (numeric.length > 0) {",
    '      const fields = numeric.sort((a, b) => Number(a) - Number(b)).map((key) => $render(input[Number(key)], debug));',
    '      return input.$type + "(" + fields.join(", ") + ")";',
    "    }",
    '    const fields = Object.keys(input).filter((key) => key !== "$type").map((key) => key + ": " + $render((input as Record<string, unknown>)[key], debug));',
    '    return input.$type + " { " + fields.join(", ") + " }";',
    "  }",
    "  return String(value);",
    "}",
  ],
  eq: [
    "function $eq(left: unknown, right: unknown): boolean {",
    "  if (left === right) return true;",
    '  if (typeof left !== "object" || typeof right !== "object" || left === null || right === null) return false;',
    "  const l = left as $Dyn & Record<string, unknown>;",
    "  const r = right as $Dyn & Record<string, unknown>;",
    '  if (typeof l.$float === "number" || typeof r.$float === "number") {',
    '    return typeof l.$float === "number" && typeof r.$float === "number" && l.$float === r.$float;',
    "  }",
    "  if (left instanceof Uint8Array || right instanceof Uint8Array) {",
    "    if (!(left instanceof Uint8Array) || !(right instanceof Uint8Array)) return false;",
    "    if (left.length !== right.length) return false;",
    "    for (let index = 0; index < left.length; index += 1) if (left[index] !== right[index]) return false;",
    "    return true;",
    "  }",
    "  if (Array.isArray(left) || Array.isArray(right)) {",
    "    if (!Array.isArray(left) || !Array.isArray(right)) return false;",
    "    return left.length === right.length && left.every((item, index) => $eq(item, right[index]));",
    "  }",
    "  if (l.$tuple || r.$tuple) {",
    "    return Boolean(l.$tuple && r.$tuple) && $eq(l.$tuple, r.$tuple);",
    "  }",
    "  if (l.$map || r.$map) {",
    "    const leftEntries = l.$map;",
    "    const rightEntries = r.$map;",
    "    if (!leftEntries || !rightEntries || leftEntries.length !== rightEntries.length) return false;",
    "    return leftEntries.every(([key, value]) => {",
    "      const entry = rightEntries.find(([candidate]) => $eq(candidate, key));",
    "      return entry !== undefined && $eq(value, entry[1]);",
    "    });",
    "  }",
    '  if (typeof l.$option === "boolean" || typeof r.$option === "boolean") {',
    '    return typeof l.$option === "boolean" && typeof r.$option === "boolean" &&',
    '      l.$option === r.$option && (l.$option === false || $eq(l.value, r.value));',
    "  }",
    "  if (l.$result || r.$result) {",
    "    return Boolean(l.$result && r.$result) && l.ok === r.ok &&",
    "      (l.ok ? $eq(l.value, r.value) : $eq(l.error, r.error));",
    "  }",
    "  if (l.$tag || r.$tag) {",
    "    if (!l.$tag || !r.$tag || l.$enum !== r.$enum || l.$tag !== r.$tag) return false;",
    "    const keys = Object.keys(l).filter((key) => /^[0-9]+$/.test(key));",
    "    if (keys.length !== Object.keys(r).filter((key) => /^[0-9]+$/.test(key)).length) return false;",
    "    return keys.every((key) => $eq(l[Number(key)], r[Number(key)]));",
    "  }",
    "  if (l.$type || r.$type) {",
    '    if (!l.$type || !r.$type || l.$type !== r.$type) return false;',
    '    const keys = Object.keys(l).filter((key) => key !== "$type");',
    '    if (keys.length !== Object.keys(r).filter((key) => key !== "$type").length) return false;',
    "    return keys.every((key) => Object.prototype.hasOwnProperty.call(r, key) && $eq(l[key], r[key]));",
    "  }",
    "  return false;",
    "}",
  ],
  len: [
    "function $len(value: unknown): number {",
    '  if (typeof value === "string") return $utf8Len(value);',
    "  if (Array.isArray(value) || value instanceof Uint8Array) return value.length;",
    "  const input = value as $Dyn;",
    "  if (input && input.$map) return input.$map.length;",
    '  $panic("len() is not implemented for this value");',
    "}",
  ],
  lenBig: [
    "function $lenBig(value: unknown): bigint {",
    '  if (typeof value === "string") return BigInt(new TextEncoder().encode(value).length);',
    "  if (Array.isArray(value) || value instanceof Uint8Array) return BigInt(value.length);",
    "  const input = value as $Dyn;",
    "  if (input && input.$map) return BigInt(input.$map.length);",
    '  $panic("len() is not implemented for this value");',
    "}",
  ],
  strSlice: [
    "function $strSlice(value: string, start: number | bigint, end: number | bigint): { readonly $result: true; readonly ok: true; readonly value: string } | { readonly $result: true; readonly ok: false; readonly error: SliceError } {",
    "  const bytes = new TextEncoder().encode(value);",
    "  const from = Number(start);",
    "  const to = Number(end);",
    "  const boundary = (position: number) =>",
    "    position === bytes.length || (position >= 0 && position < bytes.length && ((bytes[position] as number) & 0xc0) !== 0x80);",
    "  if (!Number.isInteger(from) || !Number.isInteger(to) || from < 0 || to < from || to > bytes.length || !boundary(from) || !boundary(to)) {",
    '    return $err({ $type: "SliceError", 0: "slice range is out of bounds or splits a UTF-8 scalar value" });',
    "  }",
    '  return $ok(new TextDecoder("utf-8").decode(bytes.subarray(from, to)));',
    "}",
  ],
  mapInsert: [
    "function $mapInsert<K, V>(map: { readonly $map: Array<readonly [K, V]> }, key: K, value: V): { readonly $option: true; readonly value: V } | { readonly $option: false } {",
    "  const index = map.$map.findIndex(([candidate]) => $eq(candidate, key));",
    "  if (index < 0) {",
    "    map.$map.push([key, value]);",
    "    return $none;",
    "  }",
    "  const previous = map.$map[index][1];",
    "  map.$map[index] = [key, value];",
    "  return $some(previous);",
    "}",
  ],
  checkedInteger: [
    "function $checkedInteger(tag: string, operation: string, left: number | bigint, right: number | bigint): { readonly $result: true; readonly ok: true; readonly value: number | bigint } | { readonly $result: true; readonly ok: false; readonly error: OverflowError } {",
    "  const bounds: Record<string, readonly [number | bigint, number | bigint]> = {",
    "    i8: [-128, 127],",
    "    i16: [-32768, 32767],",
    "    i32: [-2147483648, 2147483647],",
    "    i64: [-9223372036854775808n, 9223372036854775807n],",
    "    u8: [0, 255],",
    "    u16: [0, 65535],",
    "    u32: [0, 4294967295],",
    "    u64: [0n, 18446744073709551615n],",
    "  };",
    "  const [min, max] = bounds[tag];",
    "  let value: number | bigint = 0;",
    '  if (operation === "div") {',
    "    if (right === 0 || right === 0n) $panic(\"division by zero\");",
    '    value = typeof left === "bigint" ? (left as bigint) / (right as bigint) : Math.trunc((left as number) / (right as number));',
    '  } else if (operation === "add") {',
    "    value = (left as number) + (right as number);",
    '  } else if (operation === "sub") {',
    "    value = (left as number) - (right as number);",
    '  } else if (operation === "mul") {',
    "    value = (left as number) * (right as number);",
    "  } else {",
    '    $panic("unsupported checked integer operation");',
    "  }",
    "  if ((value as number) < (min as number) || (value as number) > (max as number)) {",
    '    return $err({ $type: "OverflowError", 0: "integer overflow in \'checked_" + operation + "\'" });',
    "  }",
    "  return $ok(value);",
    "}",
  ],
};
const BIG_INTEGER_TAGS = new Set(["I64", "U64"]);
const USIZE_TAGS = new Set(["Usize", "Isize"]);

function isArbitraryIntegerTag(tag: string): boolean {
  return INTEGER_TAGS.has(tag) || USIZE_TAGS.has(tag);
}

function integerWidth(tag: string): { readonly width: number; readonly signed: boolean } | undefined {
  switch (tag) {
    case "I8":
      return { width: 8, signed: true };
    case "I16":
      return { width: 16, signed: true };
    case "I32":
      return { width: 32, signed: true };
    case "I64":
      return { width: 64, signed: true };
    case "U8":
      return { width: 8, signed: false };
    case "U16":
      return { width: 16, signed: false };
    case "U32":
      return { width: 32, signed: false };
    case "U64":
      return { width: 64, signed: false };
    default:
      return undefined;
  }
}

class JavaScriptGenerator {
  private readonly ir: ModuleIr;
  private readonly ast: AstModule;
  private readonly hir: HirModule;
  private readonly backend: "js" | "ts";
  private readonly sourceText: string | undefined;
  private readonly moduleSpecifiers: Readonly<Record<string, string>> | undefined;
  private readonly transpileTypeScript: TypeScriptTranspiler | undefined;
  private readonly annotations = new Map<string, SemanticType>();
  private readonly helpers = new Set<HelperName>();
  private readonly looseTypes = new Set<LooseTypeName>();
  private readonly diagnostics: CodegenDiagnostic[] = [];
  private readonly reported = new Set<string>();
  private readonly scriptImports: string[] = [];
  private readonly scriptDeclarations = new Map<string, string>();
  private readonly scriptArtifacts: CodegenArtifact[] = [];
  private tempIndex = 0;
  private defaultExpression: Expr | undefined;
  private currentSuccessWrap = false;

  private expressionDepth = 0;

  private expressionLoopBases: number[] = [];

  private loopDepth = 0;
  private loopSignals: Array<{ break: boolean; continue: boolean }> = [];

  private usesReturnSignal = false;

  constructor(
    ir: ModuleIr,
    sourceText?: string,
    moduleSpecifiers?: Readonly<Record<string, string>>,
    backend: "js" | "ts" = "js",
    transpileTypeScript?: TypeScriptTranspiler,
  ) {
    this.ir = ir;
    this.sourceText = sourceText;
    this.moduleSpecifiers = moduleSpecifiers;
    this.backend = backend;
    this.transpileTypeScript = transpileTypeScript;
    if (!isRecord(ir.ast) || !isRecord(ir.hir)) {
      throw new CodegenRequestError("Typed module AST and HIR must be objects.");
    }
    if (!Array.isArray(ir.ast["items"]) || !Array.isArray(ir.hir["items"])) {
      throw new CodegenRequestError("Typed module AST and HIR must contain item arrays.");
    }
    this.ast = ir.ast as unknown as AstModule;
    this.hir = ir.hir as unknown as HirModule;
    if (!Array.isArray(this.hir.imports) || !isRecord(this.hir.exports)) {
      throw new CodegenRequestError("Typed module HIR is missing imports or exports.");
    }
    for (const annotation of ir.annotations) {
      this.annotations.set(spanKey(annotation.span), annotation.ty);
    }
    for (const item of this.ast.items) {
      const kind = variantOf(item.kind);
      if (kind === "Export") {
        const declaration = structPayload<{ form: unknown }>(item.kind, "Export");
        if (variantOf(declaration.form) === "DefaultValue") {
          this.defaultExpression = payloadOf(declaration.form) as Expr;
        }
      }
    }
  }

  private get isTs(): boolean {
    return this.backend === "ts";
  }

  private get backendName(): string {
    return this.backend === "ts" ? "TypeScript" : "JavaScript";
  }

  private get optionStyle(): "null" | "objects" {
    return this.isTs ? "objects" : "null";
  }

  private astType(type: AstType | null | undefined, span: Span): string {
    return astTypeToTs(
      type ?? { kind: "Unit", span },
      this.ir.integerPolicy,
      this.optionStyle,
    );
  }

  private useLooseType(name: LooseTypeName): void {
    this.looseTypes.add(name);
  }

  private emitLooseTypes(): string {
    if (!this.isTs) {
      return "";
    }
    for (const item of this.ast.items) {
      if (referencesTypePath(item.kind, "Task")) {
        this.useLooseType("AstraTask");
      }
      if (referencesTypePath(item.kind, "Channel")) {
        this.useLooseType("AstraChannel");
      }
      for (const name of BUILTIN_ERROR_TYPES) {
        if (referencesTypePath(item.kind, name)) {
          this.useLooseType(name);
        }
      }
    }

    for (const name of this.helpers) {
      const text = this.helperText(name);
      if (text.includes("$Dyn")) {
        this.useLooseType("$Dyn");
      }
      if (text.includes("AstraTask")) {
        this.useLooseType("AstraTask");
      }
      if (text.includes("AstraChannel")) {
        this.useLooseType("AstraChannel");
      }
      for (const loose of BUILTIN_ERROR_TYPES) {
        if (new RegExp(`\\b${loose}\\b`).test(text)) {
          this.useLooseType(loose);
        }
      }
    }
    const lines = LOOSE_TYPE_ORDER.filter((name) => this.looseTypes.has(name)).map(
      (name) => LOOSE_TYPE_DECLARATIONS[name],
    );
    return lines.join("\n\n");
  }

  private helperText(name: HelperName): string {
    if (!this.isTs) {
      return HELPERS[name].join("\n");
    }
    const override = TS_HELPER_OVERRIDES[name];
    if (override !== undefined) {
      return override.join("\n");
    }
    const signature = TS_HELPER_SIGNATURES[name] ?? HELPERS[name][0] ?? "";
    return [signature, ...HELPERS[name].slice(1)].join("\n");
  }

  generate(): CodegenResult {
    if (this.hir.imports.length > 0 && this.moduleSpecifiers === undefined) {
      this.unsupported(
        this.hir.imports[0]?.span,
        "Module imports require output module specifiers, which this build did not provide.",
      );
    }
    for (const scope of this.ir.manualMemoryScopes) {
      this.diagnostic(
        "NS8003",
        `Manual memory is not supported by the ${this.backendName} backend.`,
        scope,
      );
    }

    for (const item of this.hir.items) {
      if (variantOf(item.declaration) === "ScriptBlock") {
        this.prepareScriptBlock(
          structPayload<ScriptBlockDecl>(item.declaration, "ScriptBlock"),
        );
      }
    }

    const bodyParts: { readonly text: string; readonly span: Span }[] = [];
    for (const item of this.hir.items) {
      const generated = this.emitItem(item);
      if (generated.length > 0) {
        bodyParts.push({ text: generated, span: item.span });
      }
    }

    const exports = this.emitExports();
    if (exports.length > 0) {
      bodyParts.push({ text: exports, span: this.hir.items[0]?.span ?? { start: 0, end: 0 } });
    }
    const body = bodyParts.map((part) => part.text);

    const imports = this.emitImports();
    const looseTypeText = this.emitLooseTypes();
    const helperText = this.emitHelpers();
    const sections: string[] = [];
    if (imports.length > 0) {
      sections.push("", imports);
    }
    if (looseTypeText.length > 0) {
      sections.push("", looseTypeText);
    }
    if (helperText.length > 0) {
      sections.push("", helperText);
    }
    if (body.length > 0) {
      sections.push("", body.join("\n\n"));
    }
    sections.push("");
    const code = sections.join("\n");
    const sourceMap =
      this.sourceText === undefined
        ? undefined
        : createSourceMap(code, this.ir.path, this.sourceText, bodyParts);

    const declarations = this.emitDeclarations();

    return {
      code,
      diagnostics: this.diagnostics,
      ...(sourceMap === undefined ? {} : { sourceMap }),
      ...(declarations.length === 0 ? {} : { declarations }),
      ...(this.scriptArtifacts.length === 0
        ? {}
        : { artifacts: this.scriptArtifacts }),
    };
  }

  private emitDeclarations(): string {
    const named = Object.entries(this.hir.exports.named);
    if (named.length === 0 && this.hir.exports.default === null) {
      return "";
    }

    const lines: string[] = [];
    for (const imported of this.hir.imports) {
      const specifier = this.moduleSpecifier(imported.module, imported.span);
      if (specifier === undefined) {
        continue;
      }
      const kind = imported.kind;
      switch (variantOf(kind)) {
        case "Named": {
          const data = structPayload<{ bindings: readonly ImportBinding[] }>(
            kind,
            "Named",
          );
          lines.push(
            `import { ${formatImportBindings(data.bindings)} } from ${JSON.stringify(specifier)};`,
          );
          break;
        }
        case "Default": {
          const data = structPayload<{ local: string }>(kind, "Default");
          lines.push(
            `import ${jsName(data.local)} from ${JSON.stringify(specifier)};`,
          );
          break;
        }
        case "DefaultNamed": {
          const data = structPayload<{
            default: string;
            bindings: readonly ImportBinding[];
          }>(kind, "DefaultNamed");
          lines.push(
            `import ${jsName(data.default)}, { ${formatImportBindings(data.bindings)} } from ${JSON.stringify(specifier)};`,
          );
          break;
        }
        case "Namespace": {
          const data = structPayload<{ alias: string }>(kind, "Namespace");
          lines.push(
            `import * as ${jsName(data.alias)} from ${JSON.stringify(specifier)};`,
          );
          break;
        }
        default:
          break;
      }
    }
    if (lines.length > 2) {
      lines.push("");
    }

    const localExportNames = new Map<string, string>();
    for (const [exported, entry] of named) {
      if (variantOf(entry.target) === "Local") {
        const local = payloadOf(entry.target);
        if (typeof local === "string") {
          localExportNames.set(local, exported);
        }
      }
    }
    if (this.hir.exports.default !== null) {
      const target = this.hir.exports.default.target;
      if (variantOf(target) === "Local") {
        const local = payloadOf(target);
        if (typeof local === "string" && !localExportNames.has(local)) {
          localExportNames.set(local, "default");
        }
      }
    }
    const localExported = (name: string): boolean => localExportNames.has(name);

    if (this.ast.items.some((item) => referencesTypePath(item.kind, "Task"))) {
      lines.push(
        "interface AstraTask<T> {",
        "  readonly $task: true;",
        "  readonly cancelled: boolean;",
        "  join(): T;",
        "  cancel(): void;",
        "  canceled(): boolean;",
        "}",
      );
    }
    if (this.ast.items.some((item) => referencesTypePath(item.kind, "Channel"))) {
      lines.push(
        "interface AstraChannel<T> {",
        "  readonly $channel: true;",
        "  readonly capacity: number;",
        "  readonly buffer: T[];",
        "  readonly closed: boolean;",
        "}",
      );
    }

    for (const item of this.ast.items) {
      this.emitDeclarationItem(item, lines, localExported);
    }

    for (const [exported, entry] of named) {
      const target = entry.target;
      if (variantOf(target) === "Local") {
        const local = payloadOf(target);
        if (typeof local === "string") {
          lines.push(
            local === exported
              ? `export { ${jsName(local)} };`
              : `export { ${jsName(local)} as ${exportName(exported)} };`,
          );
        }
        continue;
      }
      if (variantOf(target) === "Reexport") {
        const data = structPayload<{ module: string; name: string }>(
          target,
          "Reexport",
        );
        const specifier = this.moduleSpecifier(data.module, entry.span);
        if (specifier === undefined) {
          continue;
        }
        lines.push(
          data.name === exported
            ? `export { ${exportName(data.name)} } from ${JSON.stringify(specifier)};`
            : `export { ${exportName(data.name)} as ${exportName(exported)} } from ${JSON.stringify(specifier)};`,
        );
        continue;
      }
      if (variantOf(target) === "Namespace") {
        const data = structPayload<{ module: string }>(target, "Namespace");
        const specifier = this.moduleSpecifier(data.module, entry.span);
        if (specifier !== undefined) {
          lines.push(
            `export * as ${exportName(exported)} from ${JSON.stringify(specifier)};`,
          );
        }
      }
    }

    for (const star of this.hir.exports.stars) {
      const specifier = this.moduleSpecifier(star.module, star.span);
      if (specifier !== undefined) {
        lines.push(`export * from ${JSON.stringify(specifier)};`);
      }
    }

    const defaultExport = this.hir.exports.default;
    if (defaultExport !== null) {
      const target = defaultExport.target;
      if (variantOf(target) === "Local") {
        const local = payloadOf(target);
        if (typeof local === "string") {
          lines.push(`export { ${jsName(local)} as default };`);
        }
      } else if (variantOf(target) === "Reexport") {
        const data = structPayload<{ module: string; name: string }>(
          target,
          "Reexport",
        );
        const specifier = this.moduleSpecifier(data.module, defaultExport.span);
        if (specifier !== undefined) {
          lines.push(
            data.name === "default"
              ? `export { default } from ${JSON.stringify(specifier)};`
              : `export { ${exportName(data.name)} as default } from ${JSON.stringify(specifier)};`,
          );
        }
      }
    }

    return `${lines.join("\n")}\n`;
  }

  private emitDeclarationItem(
    item: AstItem,
    lines: string[],
    localExported: (name: string) => boolean,
  ): void {
    const declaration = item.kind;
    switch (variantOf(declaration)) {
      case "Function": {
        const data = structPayload<{
          name: string;
          type_parameters: readonly string[];
          parameters: readonly { readonly name: string; readonly ty: AstType }[];
          return_type: AstType | null;
          error_set: readonly TypePath[];
        }>(declaration, "Function");
        if (!localExported(data.name)) {
          return;
        }
        const typeParameters =
          data.type_parameters.length === 0
            ? ""
            : `<${data.type_parameters.join(", ")}>`;
        const parameters = data.parameters
          .map(
            (parameter) =>
              `${jsName(parameter.name)}: ${this.astType(parameter.ty, item.span)}`,
          )
          .join(", ");
        const returnType =
          data.error_set.length === 0
            ? this.astType(data.return_type ?? { kind: "Unit", span: item.span }, item.span)
            : taggedResultType(
                this.astType(data.return_type ?? { kind: "Unit", span: item.span }, item.span),
                data.error_set.map((error) => jsName(error.segments[error.segments.length - 1] ?? "unknown")).join(" | "),
              );
        lines.push(
          `declare function ${jsName(data.name)}${typeParameters}(${parameters}): ${returnType};`,
        );
        return;
      }
      case "Constant": {
        const data = structPayload<{
          name: string;
          ty: AstType;
        }>(declaration, "Constant");
        if (localExported(data.name)) {
          lines.push(
            `declare const ${jsName(data.name)}: ${this.astType(data.ty, item.span)};`,
          );
        }
        return;
      }
      case "Struct": {
        const data = structPayload<StructDecl>(declaration, "Struct");
        lines.push(...structDeclarationLines(data, this.ir.integerPolicy, this.optionStyle));
        return;
      }
      case "Enum": {
        const data = structPayload<EnumDecl>(declaration, "Enum");
        lines.push(...enumDeclarationLines(data, this.ir.integerPolicy, this.optionStyle));
        return;
      }
      case "TypeAlias": {
        const data = structPayload<{
          name: string;
          type_parameters: readonly string[];
          ty: AstType;
        }>(declaration, "TypeAlias");
        const typeParameters =
          data.type_parameters.length === 0
            ? ""
            : `<${data.type_parameters.join(", ")}>`;
        lines.push(
          `type ${jsName(data.name)}${typeParameters} = ${this.astType(data.ty, item.span)};`,
        );
        return;
      }
      default:
        return;
    }
  }

  private emitItem(item: HirItem): string {
    const declaration = item.declaration;
    switch (variantOf(declaration)) {
      case "Function":
        return this.emitFunction(structPayload(declaration, "Function"));
      case "Struct":
        return this.emitStruct(structPayload(declaration, "Struct"));
      case "Enum":
        return this.emitEnum(structPayload(declaration, "Enum"));
      case "Constant":
        return this.emitConstant(structPayload(declaration, "Constant"));
      case "TypeAlias": {
        if (!this.isTs) {
          return "";
        }
        const data = structPayload<TypeAliasDecl>(declaration, "TypeAlias");
        const typeParameters =
          data.type_parameters.length === 0
            ? ""
            : `<${data.type_parameters.join(", ")}>`;
        return `type ${jsName(data.name)}${typeParameters} = ${this.astType(data.ty, data.span)};`;
      }
      case "ForeignModule":
        this.unsupported(
          item.span,
          `Foreign module '${item.name}' is not supported by the ${this.backendName} backend; use the native C target for FFI.`,
        );
        return "";
      case "ScriptBlock":
        return this.scriptDeclarations.get(item.name) ?? "";
      default:
        this.unsupported(item.span, "Unknown resolved declaration kind.");
        return "";
    }
  }

  private emitFunction(declaration: {
    readonly name: string;
    readonly type_parameters?: readonly string[];
    readonly parameters: readonly {
      readonly name: string;
      readonly ty: AstType;
    }[];
    readonly body: Block;
    readonly return_type?: AstType | null;
    readonly error_set?: readonly TypePath[];
    readonly span?: Span;
  }): string {
    const name = jsName(declaration.name);
    const span = declaration.span ?? { start: 0, end: 0 };
    const typeParameters =
      this.isTs && (declaration.type_parameters?.length ?? 0) > 0
        ? `<${(declaration.type_parameters ?? []).join(", ")}>`
        : "";
    const parameters = this.isTs
      ? declaration.parameters.map(
          (parameter) => `${jsName(parameter.name)}: ${this.astType(parameter.ty, span)}`,
        )
      : declaration.parameters.map((parameter) => jsName(parameter.name));
    const fallible = (declaration.error_set?.length ?? 0) > 0;
    const returnTypeText = fallible
      ? taggedResultType(
          this.astType(declaration.return_type ?? null, span),
          (declaration.error_set ?? [])
            .map((error) => jsName(error.segments[error.segments.length - 1] ?? "unknown"))
            .join(" | "),
        )
      : this.astType(declaration.return_type ?? null, span);
    const wrapsSuccess = fallible && !isResultReturnType(declaration.return_type);
    const previousWrap = this.currentSuccessWrap;
    this.currentSuccessWrap = wrapsSuccess;
    const previousReturnSignal = this.usesReturnSignal;
    this.usesReturnSignal = false;
    let body = this.emitBlock(declaration.body, "statement");
    const usesReturnSignal = this.usesReturnSignal;
    this.usesReturnSignal = previousReturnSignal;

    if (wrapsSuccess) {
      const statements = declaration.body.statements;
      const last = statements[statements.length - 1];
      if (last === undefined || variantOf(last.kind) !== "Return") {
        this.useHelper("ok");
        body = `${body.slice(0, -1)}  return $ok(undefined);\n}`;
      }
    }
    if (containsVariant(declaration.body, "Spawn")) {
      this.useHelper("taskScope");
      const scope = this.temporary("taskScope");
      body = [
        "{",
        `  const ${scope} = $enterTaskScope();`,
        `  try ${body}`,
        "  finally {",
        `    $exitTaskScope(${scope});`,
        "  }",
        "}",
      ].join("\n");
    }
    this.currentSuccessWrap = previousWrap;
    const signature = this.isTs
      ? `function ${name}${typeParameters}(${parameters.join(", ")}): ${returnTypeText}`
      : `function ${name}(${parameters.join(", ")})`;
    if (!fallible) {
      if (!usesReturnSignal) {
        return `${signature} ${body}`;
      }
      const caught = this.temporary("caught");
      if (this.isTs) {
        return [
          `${signature} {`,
          `  try ${body}`,
          `  catch (${caught}) {`,
          `    const signal = ${caught} as { readonly $return?: boolean; readonly value?: unknown } | null | undefined;`,
          `    if (signal !== null && signal !== undefined && signal.$return) return signal.value as unknown as ${returnTypeText};`,
          `    throw ${caught};`,
          "  }",
          "}",
        ].join("\n");
      }
      return [
        `${signature} {`,
        `  try ${body}`,
        `  catch (${caught}) {`,
        `    if (${caught} && ${caught}.$return) return ${caught}.value;`,
        `    throw ${caught};`,
        "  }",
        "}",
      ].join("\n");
    }

    const caught = this.temporary("caught");
    if (this.isTs) {
      return [
        `${signature} {`,
        `  try ${body}`,
        `  catch (${caught}) {`,
        `    const sentinel = ${caught} as { readonly $failure?: boolean; readonly $return?: boolean; readonly error?: unknown; readonly value?: unknown } | null | undefined;`,
        `    if (sentinel !== null && sentinel !== undefined && sentinel.$return) return sentinel.value as unknown as ${returnTypeText};`,
        `    if (sentinel !== null && sentinel !== undefined && sentinel.$failure) return ${caught} as unknown as ${returnTypeText};`,
        `    throw ${caught};`,
        "  }",
        "}",
      ].join("\n");
    }
    return [
      `${signature} {`,
      `  try ${body}`,
      `  catch (${caught}) {`,
      `    if (${caught} && ${caught}.$return) return ${caught}.value;`,
      `    if (${caught} && ${caught}.$failure) return ${caught};`,
      `    throw ${caught};`,
      "  }",
      "}",
    ].join("\n");
  }

  private emitStruct(declaration: StructDecl): string {
    const name = jsName(declaration.name);
    const fields = declaration.fields;
    const typeParameters = declaration.type_parameters;
    const parametersText =
      typeParameters.length === 0 ? "" : `<${typeParameters.join(", ")}>`;
    if (this.isTs) {

      const typeLines = structDeclarationLines(
        declaration,
        this.ir.integerPolicy,
        this.optionStyle,
      ).slice(0, -1);
      if (variantOf(fields) === "Named") {
        const named = payloadOf(fields) as readonly {
          readonly name: string;
          readonly ty: AstType;
        }[];
        const signature = named
          .map(
            (field) =>
              `${objectKey(field.name)}: ${this.astType(field.ty, declaration.span)}`,
          )
          .join("; ");
        const body = named
          .map(
            (field) => `    ${objectKey(field.name)}: fields${memberAccess(field.name)},`,
          )
          .join("\n");
        return [
          ...typeLines,
          `function ${name}${parametersText}(fields: { ${signature} }): ${name}${parametersText} {`,
          "  return {",
          `    $type: ${JSON.stringify(declaration.name)},`,
          body,
          "  };",
          "}",
        ]
          .filter((line) => line.length > 0)
          .join("\n");
      }

      const tuple = payloadOf(fields) as readonly AstType[];
      const parameters = tuple.map(
        (field, index) => `value${index}: ${this.astType(field, declaration.span)}`,
      );
      const body = tuple
        .map((_, index) => `    ${index}: value${index},`)
        .join("\n");
      return [
        ...typeLines,
        `function ${name}${parametersText}(${parameters.join(", ")}): ${name}${parametersText} {`,
        "  return {",
        `    $type: ${JSON.stringify(declaration.name)},`,
        body,
        "  };",
        "}",
      ]
        .filter((line) => line.length > 0)
        .join("\n");
    }
    if (variantOf(fields) === "Named") {
      const named = payloadOf(fields) as readonly { readonly name: string }[];
      const body = named
        .map((field) => `    ${objectKey(field.name)}: fields${memberAccess(field.name)},`)
        .join("\n");
      return [
        `function ${name}(fields) {`,
        "  return {",
        `    $type: ${JSON.stringify(declaration.name)},`,
        body,
        "  };",
        "}",
      ]
        .filter((line) => line.length > 0)
        .join("\n");
    }

    const tuple = payloadOf(fields) as readonly unknown[];
    const parameters = tuple.map((_, index) => `value${index}`);
    const body = tuple
      .map((_, index) => `    ${index}: value${index},`)
      .join("\n");
    return [
      `function ${name}(${parameters.join(", ")}) {`,
      "  return {",
      `    $type: ${JSON.stringify(declaration.name)},`,
      body,
      "  };",
      "}",
    ]
      .filter((line) => line.length > 0)
      .join("\n");
  }

  private emitEnum(declaration: EnumDecl): string {
    const name = jsName(declaration.name);
    const variants = declaration.variants.map((variant) => {
      const fields = variant.fields;
      if (fields === null || fields.length === 0) {
        return `  ${objectKey(variant.name)}: Object.freeze({ $enum: ${JSON.stringify(declaration.name)}, $tag: ${JSON.stringify(variant.name)} }),`;
      }
      const parameters = fields.map((_, index) => `value${index}`);
      const values = fields.map((_, index) => `    ${index}: value${index},`).join("\n");
      return [
        `  ${objectKey(variant.name)}: (${parameters.join(", ")}) => ({`,
        `    $enum: ${JSON.stringify(declaration.name)},`,
        `    $tag: ${JSON.stringify(variant.name)},`,
        values,
        "  }),",
      ]
        .filter((line) => line.length > 0)
        .join("\n");
    });
    if (this.isTs) {

      const declarationLines = enumDeclarationLines(
        declaration,
        this.ir.integerPolicy,
        this.optionStyle,
      );
      const instanceType = declarationLines[0] ?? `type ${name} = never;`;
      const constructorType = [
        "{",
        ...declarationLines.slice(2, -1),
        "}",
      ].join("\n");
      return [
        instanceType,
        `const ${name}: ${constructorType} = Object.freeze({`,
        variants.join("\n"),
        "});",
      ].join("\n");
    }
    return [
      `const ${name} = Object.freeze({`,
      variants.join("\n"),
      "});",
    ].join("\n");
  }

  private emitConstant(declaration: {
    readonly name: string;
    readonly ty: AstType;
    readonly value: Expr;
    readonly span: Span;
  }): string {
    if (this.isTs) {
      return `const ${jsName(declaration.name)}: ${this.astType(declaration.ty, declaration.span)} = ${this.emitExpr(declaration.value)};`;
    }
    return `const ${jsName(declaration.name)} = ${this.emitExpr(declaration.value)};`;
  }

  private prepareScriptBlock(block: ScriptBlockDecl): void {
    const transpiling = block.language === "ts";
    if (block.language !== "js" && !transpiling) {
      this.diagnostic(
        "NS7008",
        `Script language '${block.language}' is not supported by the ${this.backendName} backend; use @script:js, @script:ts, @script:c, or a matching target backend.`,
        block.header_span,
      );
      return;
    }
    if (transpiling && (this.backend !== "ts" || this.transpileTypeScript === undefined)) {
      this.diagnostic(
        "NS7008",
        this.backend === "ts"
          ? `Script block '${block.name}' is TypeScript, but no pinned TypeScript transpiler is available. A missing transpiler is a hard error; there is no best-effort fallback.`
          : `Script language 'ts' is not supported by the ${this.backendName} backend; use @script:js or a matching target backend.`,
        block.header_span,
      );
      return;
    }
    if (block.raw_source.trim().length === 0) {
      this.diagnostic(
        "NS7008",
        `Script block '${block.name}' has no implementation source.`,
        block.body_span,
      );
      return;
    }

    let bodySource = block.raw_source;
    let transpiledMap: string | undefined;
    if (transpiling) {

      const bodyStartLine =
        this.sourceText === undefined
          ? 0
          : lineAt(this.sourceText, block.body_span.start);
      const outcome = this.transpileTypeScript!(
        `${"\n".repeat(bodyStartLine)}${block.raw_source}`,
      );
      if (!outcome.ok) {
        this.diagnostic(
          "NS7008",
          `Script block '${block.name}' failed to transpile with the pinned TypeScript compiler: ${outcome.message}`,
          block.header_span,
        );
        return;
      }

      bodySource = outcome.code.replace(/\n?\/\/# sourceMappingURL=[^\n]*\n?$/, "\n");
      transpiledMap = outcome.sourceMap;
    }

    for (const exported of block.exports) {
      if (!hasEsmExport(bodySource, exported.name)) {
        this.diagnostic(
          "NS7008",
          `Script block '${block.name}' does not export declared function '${exported.name}'.`,
          exported.span,
        );
      }
    }

    const artifactPath = scriptArtifactPath(this.ir.path, block.name);
    const artifactFile = artifactPath.split("/").pop() ?? artifactPath;
    const alias = `$script_${block.name}`;
    this.scriptImports.push(
      `import * as ${alias} from ${JSON.stringify(`./${artifactFile}`)};`,
    );

    const members = block.exports
      .map((exported) =>
        indent(
          `${objectKey(exported.name)}: ${this.emitScriptExport(alias, exported)},`,
        ),
      )
      .join("\n");
    this.scriptDeclarations.set(
      block.name,
      [`const ${jsName(block.name)} = {`, members, "};"]
        .filter((line) => line.length > 0)
        .join("\n"),
    );

    const code = scriptModuleCode(this.ir.path, block, bodySource);
    const sourceMap =
      this.sourceText === undefined
        ? undefined
        : transpiling
          ? composeTranspiledScriptSourceMap(
              this.ir.path,
              this.sourceText,
              transpiledMap,
            )
          : createScriptSourceMap(
              code,
              this.ir.path,
              this.sourceText,
              block.body_span,
            );
    this.scriptArtifacts.push({
      path: artifactPath,
      code,
      ...(sourceMap === undefined ? {} : { sourceMap }),
      declarations: scriptDeclarations(block, this.ir.integerPolicy),
    });
  }

  private emitScriptExport(alias: string, exported: ScriptExport): string {
    const parameters = this.isTs
      ? exported.parameters.map(
          (parameter, index) =>
            `arg${index}: ${astTypeToTs(parameter.ty, this.ir.integerPolicy, this.optionStyle)}`,
        )
      : exported.parameters.map((_, index) => `arg${index}`);
    const argumentsText = exported.parameters
      .map((parameter, index) =>
        astBoundaryToJavaScript(`arg${index}`, parameter.ty),
      )
      .join(", ");
    const invocation = `${alias}${memberAccess(exported.name)}(${argumentsText})`;
    const returnType = exported.return_type ?? undefined;
    const fallible =
      exported.error_set.length > 0 && !isResultReturnType(returnType);
    if (isOptionType(returnType) || exported.parameters.some((parameter) => isOptionType(parameter.ty))) {
      this.useHelper("none");
      this.useHelper("some");
    }
    const returnAnnotation = fallible
      ? taggedResultType(
          this.astType(returnType ?? null, exported.span),
          jsName(scriptErrorTypeName(exported.error_set[0])),
        )
      : this.astType(returnType ?? null, exported.span);
    const typedSignature = this.isTs
      ? `(${parameters.join(", ")}): ${returnAnnotation} =>`
      : `(${parameters.join(", ")}) =>`;

    if (!fallible) {
      return `${typedSignature} ${astBoundaryFromJavaScript(
        invocation,
        returnType,
        this.ir.integerPolicy,
        this.optionStyle,
      )}`;
    }

    this.useHelper("ok");
    this.useHelper("err");
    const errorType = scriptErrorTypeName(exported.error_set[0]);
    if (this.isTs) {
      return [
        `${typedSignature} {`,
        "  try {",
        `    return $ok(${astBoundaryFromJavaScript(invocation, returnType, this.ir.integerPolicy, this.optionStyle)});`,
        "  } catch (error) {",
        "    const message = error as { readonly message?: unknown } | null | undefined;",
        `    return $err({ $type: ${JSON.stringify(errorType)}, 0: String(message !== null && message !== undefined && message.message !== undefined ? message.message : error) });`,
        "  }",
        "}",
      ].join("\n");
    }
    return [
      `${typedSignature} {`,
      "  try {",
      `    return $ok(${astBoundaryFromJavaScript(invocation, returnType, this.ir.integerPolicy, this.optionStyle)});`,
      "  } catch (error) {",
      `    return $err({ $type: ${JSON.stringify(errorType)}, 0: String(error && error.message !== undefined ? error.message : error) });`,
      "  }",
      "}",
    ].join("\n");
  }

  private emitImports(): string {
    const lines: string[] = [...this.scriptImports];
    for (const imported of this.hir.imports) {
      const specifier = this.moduleSpecifier(imported.module, imported.span);
      if (specifier === undefined) {
        continue;
      }
      const kind = imported.kind;
      switch (variantOf(kind)) {
        case "SideEffect":
          lines.push(`import ${JSON.stringify(specifier)};`);
          break;
        case "Namespace": {
          const data = structPayload<{ alias: string }>(kind, "Namespace");
          lines.push(
            `import * as ${jsName(data.alias)} from ${JSON.stringify(specifier)};`,
          );
          break;
        }
        case "Named": {
          const data = structPayload<{ bindings: readonly ImportBinding[] }>(
            kind,
            "Named",
          );
          lines.push(
            `import { ${formatImportBindings(data.bindings)} } from ${JSON.stringify(specifier)};`,
          );
          break;
        }
        case "Default": {
          const data = structPayload<{ local: string }>(kind, "Default");
          lines.push(`import ${jsName(data.local)} from ${JSON.stringify(specifier)};`);
          break;
        }
        case "DefaultNamed": {
          const data = structPayload<{
            default: string;
            bindings: readonly ImportBinding[];
          }>(kind, "DefaultNamed");
          lines.push(
            `import ${jsName(data.default)}, { ${formatImportBindings(data.bindings)} } from ${JSON.stringify(specifier)};`,
          );
          break;
        }
        default:
          this.unsupported(imported.span, "Unknown import form.");
          break;
      }
    }
    return lines.join("\n");
  }

  private moduleSpecifier(module: string, span: Span): string | undefined {
    const specifier = this.moduleSpecifiers?.[module];
    if (specifier === undefined) {
      this.unsupported(
        span,
        `No output module specifier was provided for '${module}'.`,
      );
    }
    return specifier;
  }

  private emitExports(): string {
    const lines: string[] = [];
    const surface = this.hir.exports;

    for (const [exported, entry] of Object.entries(surface.named)) {
      if (entry.kind === "Type" && !this.isTs) {

        continue;
      }
      const target = entry.target;
      const targetKind = variantOf(target);
      if (targetKind === "Local") {
        const local = payloadOf(target);
        if (typeof local !== "string") {
          this.unsupported(entry.span, `Invalid local export target for '${exported}'.`);
          continue;
        }
        const localJs = jsName(local);
        const exportedJs = exportName(exported);
        lines.push(
          localJs === exported
            ? `export { ${localJs} };`
            : `export { ${localJs} as ${exportedJs} };`,
        );
        continue;
      }
      if (targetKind === "Reexport") {
        const data = structPayload<{ module: string; name: string }>(
          target,
          "Reexport",
        );
        const specifier = this.moduleSpecifier(data.module, entry.span);
        if (specifier === undefined) {
          continue;
        }
        const imported = exportName(data.name);
        const exportedJs = exportName(exported);
        lines.push(
          imported === exportedJs
            ? `export { ${imported} } from ${JSON.stringify(specifier)};`
            : `export { ${imported} as ${exportedJs} } from ${JSON.stringify(specifier)};`,
        );
        continue;
      }
      if (targetKind === "Namespace") {
        const data = structPayload<{ module: string }>(target, "Namespace");
        const specifier = this.moduleSpecifier(data.module, entry.span);
        if (specifier !== undefined) {
          lines.push(
            `export * as ${exportName(exported)} from ${JSON.stringify(specifier)};`,
          );
        }
        continue;
      }
      this.unsupported(
        entry.span,
        `Unsupported export target for '${exported}'.`,
      );
    }

    const defaultExport = surface.default;
    if (defaultExport !== null) {
      const target = defaultExport.target;
      if (variantOf(target) === "Local") {
        const local = payloadOf(target);
        if (typeof local === "string") {
          lines.push(`export default ${jsName(local)};`);
        } else {
          this.unsupported(defaultExport.span, "Invalid default export target.");
        }
      } else if (variantOf(target) === "Expression") {
        if (this.defaultExpression === undefined) {
          this.unsupported(
            defaultExport.span,
            "The default export expression was missing from the typed AST.",
          );
        } else {
          lines.push(`export default ${this.emitExpr(this.defaultExpression)};`);
        }
      } else if (variantOf(target) === "Reexport") {
        const data = structPayload<{ module: string; name: string }>(
          target,
          "Reexport",
        );
        const specifier = this.moduleSpecifier(data.module, defaultExport.span);
        if (specifier !== undefined) {
          lines.push(
            data.name === "default"
              ? `export { default } from ${JSON.stringify(specifier)};`
              : `export { ${exportName(data.name)} as default } from ${JSON.stringify(specifier)};`,
          );
        }
      } else {
        this.unsupported(
          defaultExport.span,
          "The JavaScript backend does not support this default export form.",
        );
      }
    }

    for (const star of surface.stars) {
      const specifier = this.moduleSpecifier(star.module, star.span);
      if (specifier !== undefined) {
        lines.push(`export * from ${JSON.stringify(specifier)};`);
      }
    }

    return lines.join("\n");
  }

  private emitHelpers(): string {
    if (this.helpers.size === 0) {
      return "";
    }
    return HELPER_ORDER.filter((name) => this.helpers.has(name))
      .map((name) => this.helperText(name))
      .join("\n\n");
  }

  private useHelper(name: HelperName): void {
    if (this.helpers.has(name)) {
      return;
    }
    this.helpers.add(name);
    for (const dependency of HELPER_DEPENDENCIES[name] ?? []) {
      this.useHelper(dependency);
    }
  }

  private emitBlock(block: Block, mode: "statement" | "value"): string {
    const inner = this.emitStatements(block.statements, mode);
    if (inner.length === 0) {
      return "{}";
    }
    return `{\n${indent(inner)}\n}`;
  }

  private emitStatements(
    statements: readonly Stmt[],
    mode: "statement" | "value",
  ): string {
    if (statements.length === 0) {
      return mode === "value" ? "return undefined;" : "";
    }

    const lines: string[] = [];
    let valueReturned = false;
    for (let index = 0; index < statements.length; index += 1) {
      const statement = statements[index];
      if (statement === undefined) {
        continue;
      }
      if (variantOf(statement.kind) === "Defer") {
        const deferred = payloadOf(statement.kind) as Stmt;
        const rest = this.emitStatements(statements.slice(index + 1), mode);
        const tryBody = rest.length === 0 ? "" : `\n${indent(rest)}\n`;
        const finallyBody = indent(this.emitStatement(deferred));
        lines.push(`try {${tryBody}} finally {\n${finallyBody}\n}`);
        return lines.join("\n");
      }

      const isLast = index === statements.length - 1;
      if (isLast && mode === "value" && variantOf(statement.kind) === "Expr") {
        const expression = payloadOf(statement.kind) as Expr;
        lines.push(`return ${this.emitExpr(expression)};`);
        valueReturned = true;
      } else {
        const emitted = this.emitStatement(statement);
        if (emitted.length > 0) {
          lines.push(emitted);
        }
      }
    }

    if (mode === "value" && !valueReturned) {
      lines.push("return undefined;");
    }
    return lines.join("\n");
  }

  private emitStatement(statement: Stmt): string {
    const kind = statement.kind;
    switch (variantOf(kind)) {
      case "Let": {
        const data = structPayload<{
          mutable: boolean;
          name: string;
          ty: AstType | null;
          value: Expr;
        }>(kind, "Let");
        const annotation =
          this.isTs && data.ty !== null
            ? `: ${this.astType(data.ty, statement.span)}`
            : "";
        return `${data.mutable ? "let" : "const"} ${jsName(data.name)}${annotation} = ${this.emitExpr(
          data.value,
        )};`;
      }
      case "Return": {
        const data = structPayload<{ value: Expr | null }>(kind, "Return");
        const value =
          data.value === null ? "undefined" : this.emitExpr(data.value);
        const wrapped = this.currentSuccessWrap ? `$ok(${value})` : value;
        if (this.expressionDepth > 0) {
          this.usesReturnSignal = true;
          if (this.currentSuccessWrap) {
            this.useHelper("ok");
          }
          return `throw { $return: true, value: ${wrapped} };`;
        }
        if (this.currentSuccessWrap) {
          this.useHelper("ok");
          return `return $ok(${value});`;
        }
        return data.value === null ? "return;" : `return ${value};`;
      }
      case "Break":
        return this.emitLoopControl("break", "$break");
      case "Continue":
        return this.emitLoopControl("continue", "$continue");
      case "Defer": {
        this.unsupported(
          statement.span,
          "A defer statement must appear in a statement block.",
        );
        return "";
      }
      case "While": {
        const data = structPayload<{ condition: Expr; body: Block }>(kind, "While");
        const condition = this.emitExpr(data.condition);
        const body = this.emitLoopBody(data.body);
        return `while (${condition}) ${body}`;
      }
      case "For": {
        const data = structPayload<{
          binding: string;
          iterable: Expr;
          body: Block;
        }>(kind, "For");
        const iterable = this.emitIterable(data.iterable);
        const body = this.emitLoopBody(data.body);
        return `for (const ${jsName(data.binding)} of ${iterable}) ${body}`;
      }
      case "Expr": {
        const expression = payloadOf(kind) as Expr;
        if (variantOf(expression.kind) === "If") {
          return this.emitIfStatement(
            structPayload<{
              condition: Expr;
              then_branch: Block;
              else_branch: unknown;
            }>(expression.kind, "If"),
          );
        }
        if (variantOf(expression.kind) === "Match") {
          const data = structPayload<{
            value: Expr;
            arms: readonly MatchArm[];
          }>(expression.kind, "Match");
          return this.emitMatchStatement(data.value, data.arms);
        }
        return `${this.emitExpr(expression)};`;
      }
      case "Scoped": {
        const data = structPayload<{
          annotation: { kind: "Mode" | "Memory"; value: string };
          statements: readonly Stmt[];
        }>(kind, "Scoped");
        const inner = this.emitStatements(data.statements, "statement");
        return inner.length === 0 ? "{}" : `{\n${indent(inner)}\n}`;
      }
      default:
        this.unsupported(statement.span, "Unknown statement kind.");
        return "";
    }
  }

  private emitIterable(expression: Expr): string {
    const type = this.typeAt(expression.span);
    if (type !== undefined && typeTag(type) === "Map") {
      this.useHelper("mapKeys");
      return `$mapKeys(${this.emitExpr(expression)})`;
    }
    const value = this.emitExpr(expression);
    if (type !== undefined && typeTag(type) === "Str") {
      return `Array.from(${value})`;
    }
    return value;
  }

  private emitExpr(expression: Expr): string {
    const kind = expression.kind;
    switch (variantOf(kind)) {
      case "Literal":
        return this.emitLiteral(payloadOf(kind) as Literal, expression.span);
      case "Identifier":
        return this.emitIdentifier(payloadOf(kind) as string, expression.span);
      case "Unary": {
        const data = structPayload<{
          operator: "Negate" | "Not";
          operand: Expr;
        }>(kind, "Unary");
        const operand = this.emitExpr(data.operand);
        if (data.operator === "Not") {
          return `!(${operand})`;
        }
        const type = this.typeAt(expression.span);
        const wrapped = type === undefined ? `-(${operand})` : this.wrapNumber(type, `-(${operand})`, expression.span);
        return wrapped ?? `-(${operand})`;
      }
      case "Binary":
        return this.emitBinary(
          structPayload<{
            operator: string;
            left: Expr;
            right: Expr;
          }>(kind, "Binary"),
          expression.span,
        );
      case "Assign": {
        const data = structPayload<{ target: Expr; value: Expr }>(kind, "Assign");
        if (variantOf(data.target.kind) === "Index") {
          const indexTarget = structPayload<{ target: Expr; index: Expr }>(
            data.target.kind,
            "Index",
          );
          const targetType = this.typeAt(indexTarget.target.span);
          const targetTag = targetType === undefined ? undefined : typeTag(targetType);
          if (targetTag === "Map") {
            this.useHelper("mapIndexSet");
            return `$mapIndexSet(${this.emitExpr(indexTarget.target)}, ${this.emitExpr(indexTarget.index)}, ${this.emitExpr(data.value)})`;
          }
          if (targetTag !== undefined && isSequence(targetTag)) {
            this.useHelper("indexSet");
            return `$indexSet(${this.emitExpr(indexTarget.target)}, ${this.emitExpr(indexTarget.index)}, ${this.emitExpr(data.value)})`;
          }
        }
        return `${this.emitExpr(data.target)} = ${this.emitExpr(data.value)}`;
      }
      case "Call":
        return this.emitCall(
          structPayload<{ callee: Expr; arguments: readonly Expr[] }>(kind, "Call"),
          expression.span,
        );
      case "Index": {
        const data = structPayload<{ target: Expr; index: Expr }>(kind, "Index");
        return this.emitIndex(data.target, data.index);
      }
      case "Member": {
        const data = structPayload<{ target: Expr; member: string }>(kind, "Member");
        return this.emitMember(data.target, data.member);
      }
      case "TypeArguments":
        return this.emitExpr(
          structPayload<{ target: Expr }>(kind, "TypeArguments").target,
        );
      case "Propagate": {
        const inner = payloadOf(kind) as Expr;
        const value = this.emitExpr(inner);
        const temporary = this.temporary("propagate");
        this.useHelper("failure");
        if (this.isTs) {

          this.useLooseType("$Dyn");
          const resultType = this.typeAt(expression.span);
          const cast =
            resultType === undefined
              ? ""
              : ` as unknown as ${semanticTypeToTs(resultType, this.ir.integerPolicy, this.optionStyle)}`;
          return [
            "(() => {",
            `  const ${temporary} = ${value} as unknown as $Dyn;`,
            `  if (${temporary} !== null && ${temporary} !== undefined && ${temporary}.$failure) throw ${temporary};`,
            `  if (${temporary} !== null && ${temporary} !== undefined && ${temporary}.$result === true && ${temporary}.ok !== true) throw $failure(${temporary}.error);`,
            `  return ${temporary} !== null && ${temporary} !== undefined && ${temporary}.$result === true ? ${temporary}.value : ${temporary};`,
            `})()${cast}`,
          ].join("\n");
        }
        return [
          "(() => {",
          `  const ${temporary} = ${value};`,
          `  if (${temporary} && ${temporary}.$failure) throw ${temporary};`,
          `  if (${temporary} && ${temporary}.$result === true && !${temporary}.ok) throw $failure(${temporary}.error);`,
          `  return ${temporary} && ${temporary}.$result === true ? ${temporary}.value : ${temporary};`,
          "})()",
        ].join("\n");
      }
      case "Array": {
        const values = payloadOf(kind) as readonly Expr[];
        return `[${values.map((value) => this.emitExpr(value)).join(", ")}]`;
      }
      case "Map": {
        const entries = payloadOf(kind) as readonly {
          readonly key: Expr;
          readonly value: Expr;
        }[];
        this.useHelper("map");
        return `$map([${entries
          .map((entry) => `[${this.emitExpr(entry.key)}, ${this.emitExpr(entry.value)}]`)
          .join(", ")}])`;
      }
      case "Tuple": {
        const values = payloadOf(kind) as readonly Expr[];
        this.useHelper("tuple");
        return `$tuple([${values.map((value) => this.emitExpr(value)).join(", ")}])`;
      }
      case "StructInit": {
        const data = structPayload<{
          path: TypePath;
          fields: readonly { readonly name: string; readonly value: Expr }[];
        }>(kind, "StructInit");
        const name = data.path.segments[data.path.segments.length - 1] ?? "Anonymous";
        const fields = data.fields
          .map(
            (field) =>
              `    ${objectKey(field.name)}: ${this.emitExpr(field.value)},`,
          )
          .join("\n");
        return `${jsName(name)}({\n${fields}\n})`;
      }
      case "Block":
        return this.emitBlockExpression(payloadOf(kind) as Block);
      case "If": {
        const data = structPayload<{
          condition: Expr;
          then_branch: Block;
          else_branch: unknown;
        }>(kind, "If");
        return this.emitIfExpression(data);
      }
      case "Match": {
        const data = structPayload<{
          value: Expr;
          arms: readonly MatchArm[];
        }>(kind, "Match");
        return this.emitMatchExpression(data.value, data.arms);
      }
      case "Spawn": {
        const block = structPayload<{ body: Block }>(kind, "Spawn").body;
        const previousWrap = this.currentSuccessWrap;
        const previousDepth = this.expressionDepth;
        const previousBases = this.expressionLoopBases;
        const previousLoopDepth = this.loopDepth;
        const previousLoopSignals = this.loopSignals;
        const previousReturnSignal = this.usesReturnSignal;
        this.currentSuccessWrap = false;
        this.expressionDepth = 0;
        this.expressionLoopBases = [];
        this.loopDepth = 0;
        this.loopSignals = [];
        this.usesReturnSignal = false;
        const body = this.emitBlock(block, "value");
        const usesReturnSignal = this.usesReturnSignal;
        this.currentSuccessWrap = previousWrap;
        this.expressionDepth = previousDepth;
        this.expressionLoopBases = previousBases;
        this.loopDepth = previousLoopDepth;
        this.loopSignals = previousLoopSignals;
        this.usesReturnSignal = previousReturnSignal;
        this.useHelper("spawn");
        if (!usesReturnSignal) {
          return `$spawn(() => ${body})`;
        }
        const caught = this.temporary("caught");
        const arrow = this.isTs
          ? [
              "{",
              `  try ${body}`,
              `  catch (${caught}) {`,
              `    const signal = ${caught} as { readonly $return?: boolean; readonly value?: unknown } | null | undefined;`,
              "    if (signal !== null && signal !== undefined && signal.$return) return signal.value as never;",
              `    throw ${caught};`,
              "  }",
              "}",
            ].join("\n")
          : [
              "{",
              `  try ${body}`,
              `  catch (${caught}) {`,
              `    if (${caught} && ${caught}.$return) return ${caught}.value;`,
              `    throw ${caught};`,
              "  }",
              "}",
            ].join("\n");
        return `$spawn(() => ${arrow})`;
      }
      case "Fail": {
        const data = structPayload<{ error: TypePath; arguments: unknown }>(
          kind,
          "Fail",
        );
        this.useHelper("fail");
        return `$fail(${this.emitFailureValue(
          data.error,
          data.arguments,
          expression.span,
        )})`;
      }
      case "Panic": {
        const message = structPayload<{ message: Expr }>(kind, "Panic").message;
        this.useHelper("panic");
        this.useHelper("render");
        return `$panic($render(${this.emitExpr(message)}))`;
      }
      case "Abort": {
        const message = structPayload<{ message: Expr }>(kind, "Abort").message;
        this.useHelper("abort");
        this.useHelper("render");
        return `$abort($render(${this.emitExpr(message)}))`;
      }
      default:
        this.unsupported(expression.span, "Unknown expression kind.");
        return "undefined";
    }
  }

  private emitLiteral(literal: Literal, span: Span): string {
    switch (variantOf(literal)) {
      case "Integer": {
        const raw = payloadOf(literal);
        const value = stripIntegerSuffix(String(raw));
        const type = this.typeAt(span);
        if (type !== undefined) {
          const tag = typeTag(type);
          if (
            BIG_INTEGER_TAGS.has(tag) ||
            (this.ir.integerPolicy === "arbitrary" && isArbitraryIntegerTag(tag))
          ) {
            return `BigInt(${JSON.stringify(value)})`;
          }
        }
        return value;
      }
      case "Float": {
        const raw = stripUnderscores(String(payloadOf(literal)));
        const type = this.typeAt(span);
        this.useHelper("float");
        if (type !== undefined && typeTag(type) === "F32") {
          return `$float(Math.fround(${raw}))`;
        }
        return `$float(${raw})`;
      }
      case "String":
        return JSON.stringify(payloadOf(literal));
      case "Bytes": {
        const bytes = payloadOf(literal) as readonly number[];
        return `new Uint8Array([${bytes.join(", ")}])`;
      }
      case "Bool":
        return payloadOf(literal) === true ? "true" : "false";
      case "Unit":
        return "undefined";
      default:
        this.unsupported(span, "Unknown literal kind.");
        return "undefined";
    }
  }

  private emitFailureValue(path: TypePath, argumentsValue: unknown, span: Span): string {
    const name = path.segments[path.segments.length - 1] ?? "";
    const values =
      variantOf(argumentsValue) === "Tuple"
        ? ((payloadOf(argumentsValue) as readonly Expr[]).map((value) =>
            this.emitExpr(value),
          ))
        : ((payloadOf(argumentsValue) as readonly {
            readonly name: string;
            readonly value: Expr;
          }[]).map((field) => this.emitExpr(field.value)));

    const item = this.hir.items.find((candidate) => candidate.name === name);
    if (item === undefined) {

      const fields = values.map((value, index) => `${index}: ${value}`);
      return `{ $type: ${JSON.stringify(name)}${fields.length > 0 ? `, ${fields.join(", ")}` : ""} }`;
    }

    if (item.kind === "Enum") {
      const declaration = structPayload<EnumDecl>(item.declaration, "Enum");
      const variant = declaration.variants[0]?.name;
      if (variant === undefined) {
        this.unsupported(span, `Failure enum '${name}' has no constructible variant.`);
        return "undefined";
      }
      const constructed = `${jsName(name)}.${objectKey(variant)}`;
      return values.length === 0 ? constructed : `${constructed}(${values.join(", ")})`;
    }

    if (variantOf(argumentsValue) === "Struct") {
      const fields = payloadOf(argumentsValue) as readonly {
        readonly name: string;
        readonly value: Expr;
      }[];
      const initializers = fields
        .map(
          (field) => `    ${objectKey(field.name)}: ${this.emitExpr(field.value)},`,
        )
        .join("\n");
      return `${jsName(name)}({\n${initializers}\n})`;
    }

    return `${jsName(name)}(${values.join(", ")})`;
  }

  private emitIdentifier(name: string, _span: Span): string {
    if (name === "None") {
      this.useHelper("none");
      return "$none";
    }
    switch (name) {
      case "print":
        this.useHelper("print");
        return "$print";
      case "println":
        this.useHelper("println");
        return "$println";
      case "dbg":
        this.useHelper("dbg");
        return "$dbg";
      case "assert":
        this.useHelper("assert");
        return "$assert";
      default:
        return jsName(name);
    }
  }

  private emitBinary(
    data: { readonly operator: string; readonly left: Expr; readonly right: Expr },
    span: Span,
  ): string {
    const left = this.emitExpr(data.left);
    const right = this.emitExpr(data.right);
    switch (data.operator) {
      case "And":
        return `(${left} && ${right})`;
      case "Or":
        return `(${left} || ${right})`;
      case "Equal":
        this.useHelper("eq");
        return `$eq(${left}, ${right})`;
      case "NotEqual":
        this.useHelper("eq");
        return `!$eq(${left}, ${right})`;
      case "Less":
        return `(${left} < ${right})`;
      case "LessEqual":
        return `(${left} <= ${right})`;
      case "Greater":
        return `(${left} > ${right})`;
      case "GreaterEqual":
        return `(${left} >= ${right})`;
      default:
        break;
    }

    const type = this.typeAt(span);
    const tag = type === undefined ? undefined : typeTag(type);
    if (
      tag !== undefined &&
      (INTEGER_TAGS.has(tag) ||
        (this.ir.integerPolicy === "arbitrary" && USIZE_TAGS.has(tag)))
    ) {
      const wrap = data.operator.endsWith("Wrap");
      const operator = data.operator.replace(/Wrap$/, "");
      return this.emitIntegerBinary(tag, operator, left, right, wrap, span);
    }
    if (tag === "F32" || tag === "F64") {
      this.useHelper("float");
      let value: string;
      switch (data.operator) {
        case "Add":
        case "AddWrap":
          value = `(${left} + ${right})`;
          break;
        case "Subtract":
        case "SubtractWrap":
          value = `(${left} - ${right})`;
          break;
        case "Multiply":
        case "MultiplyWrap":
          value = `(${left} * ${right})`;
          break;
        case "Divide":
        case "DivideWrap":
          value = `(${left} / ${right})`;
          break;
        case "Remainder":
          value = `(${left} % ${right})`;
          break;
        default:
          this.unsupported(span, `Unsupported numeric operator '${data.operator}'.`);
          return "undefined";
      }
      return tag === "F32" ? `$float(Math.fround(${value}))` : `$float(${value})`;
    }

    switch (data.operator) {
      case "Add":
      case "AddWrap":
        return `(${left} + ${right})`;
      case "Subtract":
      case "SubtractWrap":
        return `(${left} - ${right})`;
      case "Multiply":
      case "MultiplyWrap":
        return `(${left} * ${right})`;
      case "Divide":
      case "DivideWrap":
        this.useHelper("div");
        return `$div(${left}, ${right})`;
      case "Remainder":
        this.useHelper("rem");
        return `$rem(${left}, ${right})`;
      default:
        this.unsupported(span, `Unsupported binary operator '${data.operator}'.`);
        return "undefined";
    }
  }

  private emitIntegerBinary(
    tag: string,
    operator: string,
    left: string,
    right: string,
    wrapping: boolean,
    span: Span,
  ): string {
    const arbitrary = this.ir.integerPolicy === "arbitrary";
    const big = BIG_INTEGER_TAGS.has(tag) || (arbitrary && isArbitraryIntegerTag(tag));
    const checked =
      !wrapping && this.ir.integerPolicy === "safe-checked";
    let raw: string;
    switch (operator) {
      case "Add":
        raw = `(${left} + ${right})`;
        break;
      case "Subtract":
        raw = `(${left} - ${right})`;
        break;
      case "Multiply":
        raw = `(${left} * ${right})`;
        break;
      case "Divide":
        if (big) {
          this.useHelper("divBig");
          raw = `$divBig(${left}, ${right})`;
        } else {
          this.useHelper("div");
          raw = `$div(${left}, ${right})`;
        }
        break;
      case "Remainder":
        if (big) {
          this.useHelper("remBig");
          raw = `$remBig(${left}, ${right})`;
        } else {
          this.useHelper("rem");
          raw = `$rem(${left}, ${right})`;
        }
        break;
      default:
        this.unsupported(span, `Unsupported integer operator '${operator}'.`);
        return "undefined";
    }

    if (arbitrary) {
      if (!wrapping) {
        return raw;
      }
      const width = integerWidth(tag);
      if (width !== undefined) {
        return `BigInt.as${width.signed ? "Int" : "Uint"}N(${width.width}, ${raw})`;
      }
      return raw;
    }

    if (checked) {
      const helper = `checked_${tag.toLowerCase()}` as HelperName;
      if (HELPER_ORDER.includes(helper)) {
        this.useHelper(helper);
        return `${helperName(helper)}(${raw})`;
      }
    }

    const helper = tag.toLowerCase() as HelperName;
    if (HELPER_ORDER.includes(helper)) {
      this.useHelper(helper);
      return `${helperName(helper)}(${raw})`;
    }
    return raw;
  }

  private wrapNumber(
    type: SemanticType,
    value: string,
    span: Span,
  ): string | undefined {
    const tag = typeTag(type);
    if (tag === "F32") {
      this.useHelper("float");
      return `$float(Math.fround(${value}))`;
    }
    if (tag === "F64") {
      this.useHelper("float");
      return `$float(${value})`;
    }
    if (tag === "Usize" || tag === "Isize") {
      return value;
    }
    if (this.ir.integerPolicy === "arbitrary" && isArbitraryIntegerTag(tag)) {
      return value;
    }
    if (this.ir.integerPolicy === "safe-checked" && INTEGER_TAGS.has(tag)) {
      const checked = `checked_${tag.toLowerCase()}` as HelperName;
      if (HELPER_ORDER.includes(checked)) {
        this.useHelper(checked);
        return `${helperName(checked)}(${value})`;
      }
    }
    const helper = tag.toLowerCase() as HelperName;
    if (HELPER_ORDER.includes(helper)) {
      this.useHelper(helper);
      return `${helperName(helper)}(${value})`;
    }
    return undefined;
  }

  private emitCall(
    data: { readonly callee: Expr; readonly arguments: readonly Expr[] },
    span: Span,
    typeArguments?: readonly AstType[],
  ): string {
    const args = data.arguments.map((argument) => this.emitExpr(argument));
    const calleeKind = variantOf(data.callee.kind);

    if (calleeKind === "Identifier") {
      const name = payloadOf(data.callee.kind) as string;
      switch (name) {
        case "print":
          this.useHelper("print");
          return `$print(${args[0] ?? "undefined"})`;
        case "println":
          this.useHelper("println");
          return `$println(${args[0] ?? "undefined"})`;
        case "dbg":
          this.useHelper("dbg");
          return `$dbg(${args[0] ?? "undefined"})`;
        case "assert":
          this.useHelper("assert");
          return `$assert(${args[0] ?? "false"})`;
        case "Some":
          if (args.length !== 1) {
            this.unsupported(span, "Some expects exactly one argument.");
            this.useHelper("none");
            return "$none";
          }
          this.useHelper("some");
          return `$some(${args[0] ?? "undefined"})`;
        case "Ok":
          this.useHelper("ok");
          return `$ok(${args[0] ?? "undefined"})`;
        case "Err":
          this.useHelper("err");
          return `$err(${args[0] ?? "undefined"})`;
        default:
          return `${this.emitExpr(data.callee)}(${args.join(", ")})`;
      }
    }

    if (calleeKind === "Member") {
      const member = structPayload<{ target: Expr; member: string }>(
        data.callee.kind,
        "Member",
      );
      const emitted = this.emitMethodCall(
        member.target,
        member.member,
        args,
        span,
        typeArguments,
      );
      if (emitted !== undefined) {
        return emitted;
      }
      return `${this.emitExpr(data.callee)}(${args.join(", ")})`;
    }

    if (calleeKind === "TypeArguments") {
      const target = structPayload<{
        readonly target: Expr;
        readonly arguments: readonly AstType[];
      }>(data.callee.kind, "TypeArguments");
      return this.emitCall(
        { callee: target.target, arguments: data.arguments },
        span,
        target.arguments,
      );
    }

    return `${this.emitExpr(data.callee)}(${args.join(", ")})`;
  }

  private emitMethodCall(
    receiver: Expr,
    method: string,
    args: readonly string[],
    span: Span,
    typeArguments?: readonly AstType[],
  ): string | undefined {
    const receiverType = this.typeAt(receiver.span);
    const tag = receiverType === undefined ? undefined : typeTag(receiverType);
    const target = this.emitExpr(receiver);

    if (
      receiverType === undefined &&
      method === "create" &&
      variantOf(receiver.kind) === "Identifier" &&
      (payloadOf(receiver.kind) as string) === "Channel"
    ) {
      this.useHelper("channel");

      if (this.isTs && typeArguments !== undefined && typeArguments.length > 0) {
        const rendered = typeArguments
          .map((argument) => this.astType(argument, span))
          .join(", ");
        return `$channel<${rendered}>()`;
      }
      return "$channel()";
    }
    if (
      receiverType === undefined &&
      method === "new" &&
      variantOf(receiver.kind) === "Identifier" &&
      (payloadOf(receiver.kind) as string) === "bytes"
    ) {
      this.useHelper("bytesNew");
      return `$bytesNew(${args[0] ?? "0"})`;
    }

    if (method === "len") {
      if (this.ir.integerPolicy === "arbitrary") {
        this.useHelper("lenBig");
        return `$lenBig(${target})`;
      }
      this.useHelper("len");
      return `$len(${target})`;
    }
    if (method === "is_empty") {
      if (this.ir.integerPolicy === "arbitrary") {
        this.useHelper("lenBig");
        return `($lenBig(${target}) === 0n)`;
      }
      this.useHelper("len");
      return `($len(${target}) === 0)`;
    }
    if (method === "push" && (tag === "Array" || tag === "Slice")) {
      return `(${target}.push(${args[0] ?? "undefined"}), undefined)`;
    }
    if (method === "get") {
      if (tag === "Map") {
        this.useHelper("mapGet");
        return `$mapGet(${target}, ${args[0] ?? "undefined"})`;
      }
      this.useHelper("getArray");
      return `$getArray(${target}, ${args[0] ?? "undefined"})`;
    }
    if (method === "insert" && tag === "Map") {
      this.useHelper("mapInsert");
      return `$mapInsert(${target}, ${args[0] ?? "undefined"}, ${args[1] ?? "undefined"})`;
    }
    if (method === "to_int" && tag === "Str") {
      if (this.ir.integerPolicy === "arbitrary") {
        this.useHelper("parseIntBig");
        return `$parseIntBig(${target})`;
      }
      this.useHelper("parseInt");
      return `$parseInt(${target})`;
    }
    if (tag === "Bytes" && method === "slice") {

      this.useHelper("byteSlice");
      return `$byteSlice(${target}, ${args[0] ?? "0"}, ${args[1] ?? "0"})`;
    }
    if (tag === "Str") {
      switch (method) {
        case "char_count":
          this.useHelper("charCount");
          return this.ir.integerPolicy === "arbitrary"
            ? `BigInt($charCount(${target}))`
            : `$charCount(${target})`;
        case "bytes":
          this.useHelper("strBytes");
          return `$strBytes(${target})`;
        case "chars":
          this.useHelper("strChars");
          return `$strChars(${target})`;
        case "byte_at":
          this.useHelper("byteAt");
          return `$byteAt(${target}, ${args[0] ?? "0"})`;
        case "char_at":
          this.useHelper("charAt");
          return `$charAt(${target}, ${args[0] ?? "0"})`;
        case "slice":
          this.useHelper("strSlice");
          return `$strSlice(${target}, ${args[0] ?? "0"}, ${args[1] ?? "0"})`;
        default:
          break;
      }
    }
    if (tag === "Named") {
      const named = payloadOf(receiverType) as readonly [string, ...unknown[]];
      if (named[0] === "Task") {
        switch (method) {
          case "join": {
            this.useHelper("taskJoin");
            const emitted = `$taskJoin(${target})`;
            if (this.isTs) {
              const resultType = this.typeAt(span);
              if (resultType !== undefined) {
                return `${emitted} as unknown as ${semanticTypeToTs(resultType, this.ir.integerPolicy, this.optionStyle)}`;
              }
            }
            return emitted;
          }
          case "cancel":
            this.useHelper("taskCancel");
            return `$taskCancel(${target})`;
          case "canceled":
            this.useHelper("taskCanceled");
            return `$taskCanceled(${target})`;
          default:
            break;
        }
      }
      if (named[0] === "Channel") {
        switch (method) {
          case "send":
            this.useHelper("channel");
            return `$channelSend(${target}, ${args[0] ?? "undefined"})`;
          case "receive":
            this.useHelper("channel");
            return `$channelReceive(${target})`;
          case "close":
            this.useHelper("channel");
            return `$channelClose(${target})`;
          default:
            break;
        }
      }
    }
    if (tag === "Option") {
      switch (method) {
        case "is_some":
          return `${target}.$option`;
        case "is_none":
          return `!${target}.$option`;
        case "unwrap":
          this.useHelper("unwrapOption");
          return `$unwrapOption(${target})`;
        case "unwrap_or": {
          if (receiverType !== undefined) {

            const optionText = semanticTypeToTs(
              receiverType,
              this.ir.integerPolicy,
              this.optionStyle,
            );
            const fallback = args[0] ?? "undefined";
            return this.isTs
              ? `((o: ${optionText}) => o.$option ? o.value : ${fallback})(${target})`
              : `(${target}.$option ? ${target}.value : ${fallback})`;
          }
          return `(${target}.$option ? ${target}.value : ${args[0] ?? "undefined"})`;
        }
        default:
          break;
      }
    }
    if (tag === "Result") {
      switch (method) {
        case "is_ok":
          return `${target}.ok`;
        case "is_err":
          return `!${target}.ok`;
        case "unwrap":
          this.useHelper("unwrapResult");
          return `$unwrapResult(${target})`;
        case "unwrap_or": {
          const fallback = args[0] ?? "undefined";
          if (receiverType !== undefined && this.isTs) {
            const resultText = semanticTypeToTs(
              receiverType,
              this.ir.integerPolicy,
              this.optionStyle,
            );
            return `((o: ${resultText}) => o.ok ? o.value : ${fallback})(${target})`;
          }
          return `(${target}.ok ? ${target}.value : ${fallback})`;
        }
        case "error":
          this.useHelper("resultError");
          return `$resultError(${target})`;
        default:
          break;
      }
    }
    if (tag !== undefined && INTEGER_TAGS.has(tag)) {
      const wrapped = this.emitIntegerMethod(tag, method, target, args[0], span);
      if (wrapped !== undefined) {
        return wrapped;
      }
    }

    return undefined;
  }

  private emitIntegerMethod(
    tag: string,
    method: string,
    receiver: string,
    argument: string | undefined,
    span: Span,
  ): string | undefined {
    const match = /^(wrapping|checked)_(add|sub|mul|div)$/.exec(method);
    if (match === null) {
      return undefined;
    }
    const kind = match[1];
    const operation = match[2];
    if (argument === undefined) {
      this.unsupported(span, `'${method}' expects one argument.`);
      return "undefined";
    }
    if (kind === "checked") {
      this.useHelper("checkedInteger");
      return `$checkedInteger(${JSON.stringify(tag.toLowerCase())}, ${JSON.stringify(operation)}, ${receiver}, ${argument})`;
    }
    const operator =
      operation === "add" ? "Add" : operation === "sub" ? "Subtract" : operation === "mul" ? "Multiply" : "Divide";
    return this.emitIntegerBinary(tag, operator, receiver, argument, true, span);
  }

  private emitIndex(target: Expr, index: Expr): string {
    const targetText = this.emitExpr(target);
    const indexText = this.emitExpr(index);
    const type = this.typeAt(target.span);
    const tag = type === undefined ? undefined : typeTag(type);
    if (tag === "Tuple") {
      this.useHelper("tupleAt");
      return `$tupleAt(${targetText}, ${indexText})`;
    }
    if (tag === "Str") {
      this.useHelper("strAt");
      return `$strAt(${targetText}, ${indexText})`;
    }
    if (tag === "Map") {
      this.useHelper("mapIndex");
      return `$mapIndex(${targetText}, ${indexText})`;
    }
    if (tag !== undefined && isSequence(tag)) {
      this.useHelper("index");
      return `$index(${targetText}, ${indexText})`;
    }
    return `${targetText}[${indexText}]`;
  }

  private emitMember(target: Expr, member: string): string {
    const targetText = this.emitExpr(target);
    const type = this.typeAt(target.span);
    const tag = type === undefined ? undefined : typeTag(type);
    if (/^[0-9]+$/.test(member)) {
      if (tag === "Tuple") {
        this.useHelper("tupleAt");
        return `$tupleAt(${targetText}, ${Number(member)})`;
      }
      return `${targetText}[${Number(member)}]`;
    }
    return `${targetText}${memberAccess(member)}`;
  }

  private emitLoopControl(keyword: "break" | "continue", signal: "$break" | "$continue"): string {
    const target = this.loopSignals[this.loopSignals.length - 1];
    const base = this.expressionLoopBases[this.expressionLoopBases.length - 1];
    if (target !== undefined && base !== undefined && this.loopDepth <= base) {
      if (keyword === "break") target.break = true;
      else target.continue = true;
      return `throw { ${signal}: true };`;
    }
    return `${keyword};`;
  }

  private emitLoopBody(block: Block): string {
    const signals = { break: false, continue: false };
    this.loopSignals.push(signals);
    this.loopDepth += 1;
    const body = this.emitBlock(block, "statement");
    this.loopDepth -= 1;
    this.loopSignals.pop();
    return this.wrapLoopSignals(body, signals);
  }

  private wrapLoopSignals(
    body: string,
    signals: { break: boolean; continue: boolean },
  ): string {
    if (!signals.break && !signals.continue) {
      return body;
    }
    const caught = this.temporary("signal");
    if (this.isTs) {
      const lines = [
        "{",
        `  try ${body}`,
        `  catch (${caught}) {`,
        `    const signal = ${caught} as { readonly $break?: boolean; readonly $continue?: boolean } | null | undefined;`,
      ];
      if (signals.break) {
        lines.push("    if (signal !== null && signal !== undefined && signal.$break) break;");
      }
      if (signals.continue) {
        lines.push(
          "    if (signal !== null && signal !== undefined && signal.$continue) continue;",
        );
      }
      lines.push(`    throw ${caught};`, "  }", "}");
      return lines.join("\n");
    }
    const lines = ["{", `  try ${body}`, `  catch (${caught}) {`];
    if (signals.break) {
      lines.push(`    if (${caught} && ${caught}.$break) break;`);
    }
    if (signals.continue) {
      lines.push(`    if (${caught} && ${caught}.$continue) continue;`);
    }
    lines.push(`    throw ${caught};`, "  }", "}");
    return lines.join("\n");
  }

  private enterExpressionBlock(): void {
    this.expressionDepth += 1;
    this.expressionLoopBases.push(this.loopDepth);
  }

  private exitExpressionBlock(): void {
    this.expressionLoopBases.pop();
    this.expressionDepth -= 1;
  }

  private emitBlockExpression(block: Block): string {
    this.enterExpressionBlock();
    const body = this.emitBlock(block, "value");
    this.exitExpressionBlock();
    return `(() => ${body})()`;
  }

  private emitIfExpression(data: {
    readonly condition: Expr;
    readonly then_branch: Block;
    readonly else_branch: unknown;
  }): string {
    this.enterExpressionBlock();
    const condition = this.emitExpr(data.condition);
    const thenBranch = this.emitBlock(data.then_branch, "value");
    let otherwise = "";
    if (data.else_branch !== null && data.else_branch !== undefined) {
      if (variantOf(data.else_branch) === "Block") {
        otherwise = ` else ${this.emitBlock(payloadOf(data.else_branch) as Block, "value")}`;
      } else {
        otherwise = ` else return ${this.emitExpr(payloadOf(data.else_branch) as Expr)};`;
      }
    }
    this.exitExpressionBlock();
    return `(() => {\n  if (${condition}) ${thenBranch}${otherwise}\n})()`;
  }

  private emitIfStatement(data: {
    readonly condition: Expr;
    readonly then_branch: Block;
    readonly else_branch: unknown;
  }): string {
    const condition = this.emitExpr(data.condition);
    const thenBranch = this.emitBlock(data.then_branch, "statement");
    let otherwise = "";
    if (data.else_branch !== null && data.else_branch !== undefined) {
      if (variantOf(data.else_branch) === "Block") {
        otherwise = ` else ${this.emitBlock(
          payloadOf(data.else_branch) as Block,
          "statement",
        )}`;
      } else {
        const alternative = payloadOf(data.else_branch) as Expr;
        otherwise =
          variantOf(alternative.kind) === "If"
            ? ` else ${this.emitIfStatement(
                structPayload<{
                  condition: Expr;
                  then_branch: Block;
                  else_branch: unknown;
                }>(alternative.kind, "If"),
              )}`
            : ` else {\n${indent(`${this.emitExpr(alternative)};`)}\n}`;
      }
    }
    return `if (${condition}) ${thenBranch}${otherwise}`;
  }

  private emitMatchExpression(value: Expr, arms: readonly MatchArm[]): string {
    this.enterExpressionBlock();
    const scrutinee = this.prepareMatchScrutinee(value);
    const lines: string[] = [...scrutinee.lines];
    let hasCatchAll = false;

    for (const arm of arms) {
      if (hasCatchAll) {
        continue;
      }
      const pattern = this.lowerPattern(
        arm.pattern,
        scrutinee.temporary,
        scrutinee.bindingType,
      );
      const body =
        variantOf(arm.body) === "Expr"
          ? `return ${this.emitExpr(payloadOf(arm.body) as Expr)};`
          : this.emitBlock(payloadOf(arm.body) as Block, "value");
      const bindings =
        pattern.bindings.length === 0 ? "" : `${indent(pattern.bindings.join("\n"))}\n`;
      if (pattern.checks.length === 0) {
        hasCatchAll = true;
        lines.push(`{\n${bindings}${indent(body)}\n}`);
      } else {
        lines.push(
          `if (${pattern.checks.join(" && ")}) {\n${bindings}${indent(body)}\n}`,
        );
      }
    }

    this.useHelper("panic");
    lines.push('$panic("non-exhaustive match");');
    this.exitExpressionBlock();
    return `(() => {\n${indent(lines.join("\n"))}\n})()`;
  }

  private emitMatchStatement(value: Expr, arms: readonly MatchArm[]): string {
    const scrutinee = this.prepareMatchScrutinee(value);
    const prelude = [...scrutinee.lines];
    const segments: string[] = [];
    let hasCatchAll = false;

    for (const arm of arms) {
      if (hasCatchAll) {
        continue;
      }
      const pattern = this.lowerPattern(
        arm.pattern,
        scrutinee.temporary,
        scrutinee.bindingType,
      );
      const body =
        variantOf(arm.body) === "Expr"
          ? `${this.emitExpr(payloadOf(arm.body) as Expr)};`
          : this.emitBlock(payloadOf(arm.body) as Block, "statement");
      const bindings =
        pattern.bindings.length === 0 ? "" : `${indent(pattern.bindings.join("\n"))}\n`;
      const guard =
        pattern.checks.length === 0
          ? ""
          : `if (${pattern.checks.join(" && ")}) `;
      if (pattern.checks.length === 0) {
        hasCatchAll = true;
      }
      segments.push(`${guard}{\n${bindings}${indent(body)}\n}`);
    }

    let chain = segments.join(" else ");
    if (!hasCatchAll) {
      this.useHelper("panic");
      const fallback = `{\n${indent('$panic("non-exhaustive match");')}\n}`;
      chain = chain.length === 0 ? fallback : `${chain} else ${fallback}`;
    }
    return chain.length === 0
      ? prelude.join("\n")
      : [...prelude, chain].join("\n");
  }

  private prepareMatchScrutinee(value: Expr): {
    readonly temporary: string;
    readonly lines: readonly string[];
    readonly bindingType: PatternBindingType | undefined;
  } {
    const temporary = this.temporary("match");
    this.useHelper("err");
    let rootSemantic = this.typeAt(value.span);

    if (rootSemantic === undefined || typeTag(rootSemantic) !== "Result") {
      const call =
        variantOf(value.kind) === "Call"
          ? (payloadOf(value.kind) as { readonly callee: Expr })
          : undefined;
      const callee = call === undefined ? undefined : this.typeAt(call.callee.span);
      if (
        callee !== undefined &&
        typeof callee === "object" &&
        "Function" in callee &&
        callee.Function.errors[0] !== undefined
      ) {
        rootSemantic = {
          Result: [callee.Function.return_type, callee.Function.errors[0]],
        };
      }
    }
    const lines = this.isTs
      ? [
          `let ${temporary} = ${this.emitExpr(value)} as unknown as $Dyn;`,
          `if (${temporary} !== null && ${temporary} !== undefined && ${temporary}.$failure) ${temporary} = $err(${temporary}.error);`,
        ]
      : [
          `let ${temporary} = ${this.emitExpr(value)};`,
          `if (${temporary} && ${temporary}.$failure) ${temporary} = $err(${temporary}.error);`,
        ];
    if (this.isTs) {
      this.useLooseType("$Dyn");
    }
    const bindingType: PatternBindingType | undefined =
      this.isTs && rootSemantic !== undefined
        ? { kind: "semantic", ty: rootSemantic }
        : undefined;
    return { temporary, lines, bindingType };
  }

  private lowerPattern(
    pattern: Pattern,
    value: string,
    bindingType?: PatternBindingType,
  ): { readonly checks: readonly string[]; readonly bindings: readonly string[] } {
    const checks: string[] = [];
    const bindings: string[] = [];
    this.lowerPatternInto(pattern, value, checks, bindings, bindingType);
    return { checks, bindings };
  }

  private renderBindingType(
    bindingType: PatternBindingType | undefined,
    span: Span,
  ): string | undefined {
    if (bindingType === undefined) {
      return undefined;
    }
    return bindingType.kind === "semantic"
      ? semanticTypeToTs(bindingType.ty, this.ir.integerPolicy, this.optionStyle)
      : this.astType(bindingType.ty, span);
  }

  private tupleElementBinding(
    bindingType: PatternBindingType | undefined,
    index: number,
  ): PatternBindingType | undefined {
    if (bindingType === undefined || bindingType.kind !== "semantic") {
      return undefined;
    }
    const tag = typeTag(bindingType.ty);
    if (tag === "Tuple") {
      const element = (typePayload(bindingType.ty) as readonly SemanticType[])[index];
      return element === undefined ? undefined : { kind: "semantic", ty: element };
    }
    if (tag === "Array" || tag === "Slice") {
      return { kind: "semantic", ty: typePayload(bindingType.ty) as SemanticType };
    }
    if (tag === "FixedArray") {
      const element = (typePayload(bindingType.ty) as readonly [SemanticType, number])[0];
      return element === undefined ? undefined : { kind: "semantic", ty: element };
    }
    return undefined;
  }

  private optionElementBinding(
    bindingType: PatternBindingType | undefined,
  ): PatternBindingType | undefined {
    if (bindingType === undefined) {
      return undefined;
    }
    if (bindingType.kind === "semantic") {
      if (typeTag(bindingType.ty) !== "Option") {
        return undefined;
      }
      const element = typePayload(bindingType.ty) as SemanticType;
      return { kind: "semantic", ty: element };
    }
    const path = astPath(bindingType.ty);
    if (path === undefined || path.segments[path.segments.length - 1] !== "Option") {
      return undefined;
    }
    const element = path.arguments[0];
    return element === undefined ? undefined : { kind: "ast", ty: element };
  }

  private resultElementBinding(
    bindingType: PatternBindingType | undefined,
    which: 0 | 1,
  ): PatternBindingType | undefined {
    if (bindingType === undefined) {
      return undefined;
    }
    if (bindingType.kind === "semantic") {
      if (typeTag(bindingType.ty) !== "Result") {
        return undefined;
      }
      const element = (typePayload(bindingType.ty) as readonly [SemanticType, SemanticType])[which];
      return element === undefined ? undefined : { kind: "semantic", ty: element };
    }
    const path = astPath(bindingType.ty);
    if (path === undefined || path.segments[path.segments.length - 1] !== "Result") {
      return undefined;
    }
    const element = path.arguments[which];
    return element === undefined ? undefined : { kind: "ast", ty: element };
  }

  private declaredFieldBinding(
    bindingType: PatternBindingType | undefined,
    name: string,
    index: number,
    kind: "struct" | "enum",
    variant?: string,
  ): PatternBindingType | undefined {
    if (bindingType === undefined) {
      return undefined;
    }
    const item = this.hir.items.find((candidate) => candidate.name === name);
    if (item === undefined) {
      return undefined;
    }
    if (kind === "enum" && item.kind === "Enum") {
      const declaration = structPayload<EnumDecl>(item.declaration, "Enum");
      const target = declaration.variants.find((entry) => entry.name === variant);
      const field = target?.fields?.[index];
      return field === undefined ? undefined : { kind: "ast", ty: field.ty };
    }
    if (kind === "struct" && item.kind === "Struct") {
      const declaration = structPayload<StructDecl>(item.declaration, "Struct");
      const fields = declaration.fields;
      if (variantOf(fields) === "Named") {
        const field = (payloadOf(fields) as readonly { readonly ty: AstType }[])[index];
        return field === undefined ? undefined : { kind: "ast", ty: field.ty };
      }
      const field = (payloadOf(fields) as readonly AstType[])[index];
      return field === undefined ? undefined : { kind: "ast", ty: field };
    }
    return undefined;
  }

  private lowerPatternInto(
    pattern: Pattern,
    value: string,
    checks: string[],
    bindings: string[],
    bindingType?: PatternBindingType,
  ): void {
    const kind = pattern.kind;
    const tag = variantOf(kind);
    switch (tag) {
      case "Wildcard":
        return;
      case "Identifier": {
        const name = payloadOf(kind);
        if (typeof name !== "string") {
          return;
        }
        if (name === "None") {
          checks.push(`${value}.$option === false`);
          return;
        }
        const rendered = this.isTs
          ? this.renderBindingType(bindingType, pattern.span)
          : undefined;
        bindings.push(
          rendered === undefined
            ? `const ${jsName(name)} = ${value};`
            : `const ${jsName(name)} = ${value} as ${rendered};`,
        );
        return;
      }
      case "Literal": {
        this.useHelper("eq");
        checks.push(
          `$eq(${value}, ${this.emitLiteral(payloadOf(kind) as Literal, pattern.span)})`,
        );
        return;
      }
      case "Tuple": {
        const items = payloadOf(kind) as readonly Pattern[];
        checks.push(`Array.isArray(${value})`);
        checks.push(`${value}.length === ${items.length}`);
        items.forEach((item, index) => {
          this.lowerPatternInto(
            item,
            `${value}[${index}]`,
            checks,
            bindings,
            this.tupleElementBinding(bindingType, index),
          );
        });
        return;
      }
      case "Constructor": {
        const data = structPayload<{
          path: TypePath;
          arguments: readonly Pattern[];
        }>(kind, "Constructor");
        const name = data.path.segments[data.path.segments.length - 1] ?? "";
        if (name === "Some") {
          checks.push(`${value}.$option === true`);
          const argument = data.arguments[0];
          if (argument !== undefined) {
            this.lowerPatternInto(
              argument,
              `${value}.value`,
              checks,
              bindings,
              this.optionElementBinding(bindingType),
            );
          }
          return;
        }
        if (name === "Ok" || name === "Err") {
          checks.push(`${value}.ok === ${name === "Ok" ? "true" : "false"}`);
          const argument = data.arguments[0];
          if (argument !== undefined) {
            this.lowerPatternInto(
              argument,
              `${value}.${name === "Ok" ? "value" : "error"}`,
              checks,
              bindings,
              this.resultElementBinding(bindingType, name === "Ok" ? 0 : 1),
            );
          }
          return;
        }
        checks.push(`${value} !== null && ${value}.$type === ${JSON.stringify(name)}`);
        data.arguments.forEach((argument, index) => {
          this.lowerPatternInto(
            argument,
            `${value}[${index}]`,
            checks,
            bindings,
            this.declaredFieldBinding(bindingType, name, index, "struct"),
          );
        });
        return;
      }
      case "Variant": {
        const data = structPayload<{
          path: TypePath;
          variant: string;
          arguments: readonly Pattern[] | null;
        }>(kind, "Variant");
        checks.push(`${value} !== null && ${value}.$tag === ${JSON.stringify(data.variant)}`);
        if (data.arguments !== null) {
          const enumName = data.path.segments[data.path.segments.length - 1] ?? "";
          data.arguments.forEach((argument, index) => {
            this.lowerPatternInto(
              argument,
              `${value}[${index}]`,
              checks,
              bindings,
              this.declaredFieldBinding(
                bindingType,
                enumName,
                index,
                "enum",
                data.variant,
              ),
            );
          });
        }
        return;
      }
      default:
        this.unsupported(pattern.span, "Unknown pattern kind.");
    }
  }

  private typeAt(span: Span): SemanticType | undefined {
    return this.annotations.get(spanKey(span));
  }

  private temporary(label: string): string {
    this.tempIndex += 1;
    return `$${label}${this.tempIndex}`;
  }

  private unsupported(span: Span | undefined, message: string): void {
    this.diagnostic("NS8001", message, span);
  }

  private diagnostic(code: string, message: string, span: Span | undefined): void {
    const key = span === undefined ? `${code}:` : `${code}:${span.start}:${span.end}`;
    if (this.reported.has(key)) {
      return;
    }
    this.reported.add(key);
    this.diagnostics.push(
      span === undefined ? { code, message } : { code, message, span },
    );
  }
}

export function generateJavaScript(request: CodegenRequest): CodegenResult {
  return generateModule({ ...request, backend: "js" });
}

export function generateModule(request: CodegenRequest): CodegenResult {
  const ir = parseModuleIr(request.ir);
  return new JavaScriptGenerator(
    ir,
    request.sourceText,
    request.moduleSpecifiers,
    request.backend === "ts" ? "ts" : "js",
    request.transpileTypeScript,
  ).generate();
}

function parseModuleIr(value: unknown): ModuleIr {
  if (!isRecord(value)) {
    throw new CodegenRequestError("Typed module request must be an object.");
  }
  const { key, path, ast, hir, annotations, manualMemoryScopes, integerPolicy } = value;
  if (
    typeof key !== "string" ||
    typeof path !== "string" ||
    !isRecord(ast) ||
    !isRecord(hir) ||
    !Array.isArray(annotations) ||
    !Array.isArray(manualMemoryScopes) ||
    (integerPolicy !== "wrapping" &&
      integerPolicy !== "safe-checked" &&
      integerPolicy !== "arbitrary")
  ) {
    throw new CodegenRequestError("Typed module request has an invalid shape.");
  }
  return value as unknown as ModuleIr;
}

function spanKey(span: Span): string {
  return `${span.start}:${span.end}`;
}

function isSequence(tag: string): boolean {
  return tag === "Array" || tag === "FixedArray" || tag === "Slice" || tag === "Bytes";
}

function taggedResultType(ok: string, error: string): string {
  return `{ readonly $result: true; readonly ok: true; readonly value: ${ok} } | { readonly $result: true; readonly ok: false; readonly error: ${error} }`;
}

function referencesTypePath(node: unknown, name: string): boolean {
  if (Array.isArray(node)) {
    return node.some((child) => referencesTypePath(child, name));
  }
  if (node !== null && typeof node === "object") {
    const path = (node as { readonly Path?: TypePath }).Path;
    if (path !== undefined && Array.isArray(path.segments)) {
      if (path.segments[path.segments.length - 1] === name) {
        return true;
      }
    }
    return Object.values(node as Record<string, unknown>).some((child) =>
      referencesTypePath(child, name),
    );
  }
  return false;
}

function astTypeToTs(
  type: AstType,
  policy: "wrapping" | "safe-checked" | "arbitrary",
  optionStyle: "null" | "objects" = "null",
): string {
  switch (variantOf(type.kind)) {
    case "Unit":
      return "void";
    case "Array":
    case "Slice":
      return `${astTypeToTs(payloadOf(type.kind) as AstType, policy, optionStyle)}[]`;
    case "FixedArray": {
      const [element] = payloadOf(type.kind) as readonly [AstType, string];
      return `${astTypeToTs(element, policy, optionStyle)}[]`;
    }
    case "Map": {
      const [key, value] = payloadOf(type.kind) as readonly [AstType, AstType];
      return `{ readonly $map: Array<[${astTypeToTs(key, policy, optionStyle)}, ${astTypeToTs(value, policy, optionStyle)}]> }`;
    }
    case "Tuple": {
      const values = payloadOf(type.kind) as readonly AstType[];
      const rendered = `[${values.map((value) => astTypeToTs(value, policy, optionStyle)).join(", ")}]`;
      return optionStyle === "objects" ? `{ readonly $tuple: ${rendered} }` : rendered;
    }
    case "Function": {
      const data = payloadOf(type.kind) as {
        readonly parameters: readonly AstType[];
        readonly return_type: AstType | null;
      };
      const parameters = data.parameters
        .map((parameter, index) => `arg${index}: ${astTypeToTs(parameter, policy, optionStyle)}`)
        .join(", ");
      return `(${parameters}) => ${astTypeToTs(
        data.return_type ?? { kind: "Unit", span: type.span },
        policy,
        optionStyle,
      )}`;
    }
    case "Path": {
      const path = payloadOf(type.kind) as TypePath;
      const name = path.segments[path.segments.length - 1] ?? "unknown";
      const argumentsText = path.arguments.map((argument) =>
        astTypeToTs(argument, policy, optionStyle),
      );
      if (path.segments.length > 1 && optionStyle === "objects") {

        const qualified = path.segments.map(jsName).join(".");
        return argumentsText.length === 0
          ? qualified
          : `${qualified}<${argumentsText.join(", ")}>`;
      }
      if (name === "Option" && argumentsText.length === 1) {
        return optionStyle === "objects"
          ? `{ readonly $option: true; readonly value: ${argumentsText[0]} } | { readonly $option: false }`
          : `(${argumentsText[0]} | null)`;
      }
      if (name === "Result" && argumentsText.length === 2) {
        return taggedResultType(argumentsText[0] ?? "unknown", argumentsText[1] ?? "unknown");
      }
      if ((name === "Array" || name === "Slice") && argumentsText.length === 1) {
        return `${argumentsText[0]}[]`;
      }
      if (name === "Channel" && argumentsText.length === 1) {
        return `AstraChannel<${argumentsText[0]}>`;
      }
      if (name === "Task" && argumentsText.length === 1) {
        return `AstraTask<${argumentsText[0]}>`;
      }
      if (name === "FixedArray" && argumentsText.length === 1) {
        return `${argumentsText[0]}[]`;
      }
      if (name === "Map" && argumentsText.length === 2) {
        return `{ readonly $map: Array<[${argumentsText[0]}, ${argumentsText[1]}]> }`;
      }
      switch (name.toLowerCase()) {
        case "i8":
        case "i16":
        case "i32":
        case "u8":
        case "u16":
        case "u32":
          return policy === "arbitrary" ? "bigint" : "number";
        case "i64":
        case "u64":
          return "bigint";
        case "usize":
        case "isize":
          return policy === "arbitrary" ? "bigint" : "number";
        case "f32":
        case "f64":
          return "number";
        case "bool":
          return "boolean";
        case "str":
          return "string";
        case "bytes":
          return "Uint8Array";
        case "unit":
          return "void";
        case "never":
          return "never";
        case "error":
        case "unknown":
          return "unknown";
        default:
          return argumentsText.length === 0
            ? jsName(name)
            : `${jsName(name)}<${argumentsText.join(", ")}>`;
      }
    }
    default:
      return "unknown";
  }
}

type PatternBindingType =
  | { readonly kind: "semantic"; readonly ty: SemanticType }
  | { readonly kind: "ast"; readonly ty: AstType };

function semanticTypeToTs(
  type: SemanticType,
  policy: "wrapping" | "safe-checked" | "arbitrary",
  optionStyle: "null" | "objects" = "null",
): string {
  if (typeof type === "string") {
    switch (type) {
      case "I8":
      case "I16":
      case "I32":
      case "U8":
      case "U16":
      case "U32":

        return policy === "arbitrary" ? "bigint" : "number";
      case "F32":
      case "F64":
        return "number";
      case "I64":
      case "U64":
        return "bigint";
      case "Usize":
      case "Isize":
        return policy === "arbitrary" ? "bigint" : "number";
      case "Bool":
        return "boolean";
      case "Str":
        return "string";
      case "Bytes":
        return "Uint8Array";
      case "Unit":
        return "void";
      case "Never":
        return "never";
      case "Error":
      case "Unknown":
      default:
        return "unknown";
    }
  }
  const tag = typeTag(type);
  const payload = typePayload(type);
  switch (tag) {
    case "Option": {
      const inner = semanticTypeToTs(payload as SemanticType, policy, optionStyle);
      return optionStyle === "objects"
        ? `{ readonly $option: true; readonly value: ${inner} } | { readonly $option: false }`
        : `(${inner} | null)`;
    }
    case "Result": {
      const [ok, error] = payload as readonly [SemanticType, SemanticType];
      return taggedResultType(
        semanticTypeToTs(ok ?? "Unknown", policy, optionStyle),
        semanticTypeToTs(error ?? "Unknown", policy, optionStyle),
      );
    }
    case "Array":
    case "Slice":
      return `${semanticTypeToTs(payload as SemanticType, policy, optionStyle)}[]`;
    case "FixedArray": {
      const [element] = payload as readonly [SemanticType, number];
      return `${semanticTypeToTs(element ?? "Unknown", policy, optionStyle)}[]`;
    }
    case "Map": {
      const [key, value] = payload as readonly [SemanticType, SemanticType];
      return `{ readonly $map: Array<[${semanticTypeToTs(key ?? "Unknown", policy, optionStyle)}, ${semanticTypeToTs(value ?? "Unknown", policy, optionStyle)}]> }`;
    }
    case "Tuple": {
      const values = payload as readonly SemanticType[];
      const rendered = `[${values.map((value) => semanticTypeToTs(value, policy, optionStyle)).join(", ")}]`;
      return optionStyle === "objects" ? `{ readonly $tuple: ${rendered} }` : rendered;
    }
    case "Function": {
      const data = payload as FunctionType;
      const parameters = data.parameters
        .map(
          (parameter, index) =>
            `arg${index}: ${semanticTypeToTs(parameter, policy, optionStyle)}`,
        )
        .join(", ");
      return `(${parameters}) => ${semanticTypeToTs(data.return_type, policy, optionStyle)}`;
    }
    case "Named": {
      const [name, args] = payload as readonly [string, readonly SemanticType[]];
      const argumentsText = args.map((argument) =>
        semanticTypeToTs(argument, policy, optionStyle),
      );
      if (name === "Option" && argumentsText.length === 1) {
        return optionStyle === "objects"
          ? `{ readonly $option: true; readonly value: ${argumentsText[0]} } | { readonly $option: false }`
          : `(${argumentsText[0]} | null)`;
      }
      if (name === "Result" && argumentsText.length === 2) {
        return taggedResultType(argumentsText[0] ?? "unknown", argumentsText[1] ?? "unknown");
      }
      if (
        (name === "Array" || name === "Slice" || name === "FixedArray") &&
        argumentsText.length === 1
      ) {
        return `${argumentsText[0]}[]`;
      }
      if (name === "Map" && argumentsText.length === 2) {
        return `{ readonly $map: Array<[${argumentsText[0]}, ${argumentsText[1]}]> }`;
      }
      if (name === "Channel" && argumentsText.length === 1) {
        return `AstraChannel<${argumentsText[0]}>`;
      }
      if (name === "Task" && argumentsText.length === 1) {
        return `AstraTask<${argumentsText[0]}>`;
      }
      return argumentsText.length === 0
        ? jsName(name)
        : `${jsName(name)}<${argumentsText.join(", ")}>`;
    }
    case "Generic":
      return jsName(payload as string);
    default:
      return "unknown";
  }
}

function structDeclarationLines(
  declaration: StructDecl,
  policy: "wrapping" | "safe-checked" | "arbitrary",
  optionStyle: "null" | "objects" = "null",
): readonly string[] {
  const name = jsName(declaration.name);
  const typeParameters = declaration.type_parameters;
  const parametersText =
    typeParameters.length === 0 ? "" : `<${typeParameters.join(", ")}>`;
  const declarationParametersText =
    typeParameters.length === 0 ? "" : `<${typeParameters.join(", ")}>`;
  const fields = declaration.fields;
  if (variantOf(fields) === "Named") {
    const named = payloadOf(fields) as readonly {
      readonly name: string;
      readonly ty: AstType;
    }[];
    const body = named
      .map(
        (field) =>
          `  ${objectKey(field.name)}: ${astTypeToTs(field.ty, policy, optionStyle)};`,
      )
      .join("\n");
    const parameters = named
      .map(
        (field) =>
          `${objectKey(field.name)}: ${astTypeToTs(field.ty, policy, optionStyle)}`,
      )
      .join("; ");
    return [
      `interface ${name}${parametersText} {`,
      `  readonly $type: ${JSON.stringify(declaration.name)};`,
      body,
      "}",
      `declare const ${name}: ${declarationParametersText === "" ? "" : `${declarationParametersText} `}(fields: { ${parameters} }) => ${name}${parametersText};`,
    ];
  }

  const tuple = payloadOf(fields) as readonly AstType[];
  const body = tuple
    .map(
      (field, index) =>
        `  ${index}: ${astTypeToTs(field, policy, optionStyle)};`,
    )
    .join("\n");
  const parameters = tuple
    .map(
      (field, index) =>
        `value${index}: ${astTypeToTs(field, policy, optionStyle)}`,
    )
    .join(", ");
  return [
    `interface ${name}${parametersText} {`,
    `  readonly $type: ${JSON.stringify(declaration.name)};`,
    body,
    "}",
    `declare const ${name}: ${declarationParametersText === "" ? "" : `${declarationParametersText} `}(${parameters}) => ${name}${parametersText};`,
  ];
}

function enumDeclarationLines(
  declaration: EnumDecl,
  policy: "wrapping" | "safe-checked" | "arbitrary",
  optionStyle: "null" | "objects" = "null",
): readonly string[] {
  const name = jsName(declaration.name);
  const typeParameters = declaration.type_parameters;
  const parametersText =
    typeParameters.length === 0 ? "" : `<${typeParameters.join(", ")}>`;
  const variants = declaration.variants.map((variant) => {
    const fields = variant.fields ?? [];
    const payload = fields
      .map(
        (field, index) =>
          `readonly ${index}: ${astTypeToTs(field.ty, policy, optionStyle)};`,
      )
      .join(" ");
    return `{ readonly $enum: ${JSON.stringify(declaration.name)}; readonly $tag: ${JSON.stringify(variant.name)};${payload.length > 0 ? ` ${payload}` : ""} }`;
  });
  const members = declaration.variants
    .map((variant, index) => {
      const fields = variant.fields ?? [];
      const variantType = variants[index];
      if (fields.length === 0) {
        return `  readonly ${objectKey(variant.name)}: ${variantType};`;
      }
      const parameters = fields
        .map(
          (field, index) =>
            `value${index}: ${astTypeToTs(field.ty, policy, optionStyle)}`,
        )
        .join(", ");
      return `  readonly ${objectKey(variant.name)}: ${declarationParameters(typeParameters)}(${parameters}) => ${variantType};`;
    })
    .join("\n");
  return [
    `type ${name}${parametersText} = ${variants.length === 0 ? "never" : variants.join(" | ")};`,
    `declare const ${name}: {`,
    members,
    "};",
  ];
}

function declarationParameters(typeParameters: readonly string[]): string {
  return typeParameters.length === 0 ? "" : `<${typeParameters.join(", ")}> `;
}

function scriptArtifactPath(modulePath: string, name: string): string {
  return `${modulePath.replace(/\.ns$/i, "")}.${name}.js`;
}

function scriptDeclarations(
  block: ScriptBlockDecl,
  policy: "wrapping" | "safe-checked" | "arbitrary",
): string {
  const lines: string[] = [];
  for (const exported of block.exports) {
    const parameters = exported.parameters
      .map(
        (parameter) =>
          `${jsName(parameter.name)}: ${astTypeToTs(parameter.ty, policy)}`,
      )
      .join(", ");
    const returnType = astTypeToTs(
      exported.return_type ?? { kind: "Unit", span: exported.span },
      policy,
    );
    lines.push(
      `export declare function ${jsName(exported.name)}(${parameters}): ${returnType};`,
    );
  }
  return `${lines.join("\n")}\n`;
}

function scriptModuleCode(
  modulePath: string,
  block: ScriptBlockDecl,
  bodySource: string = block.raw_source,
): string {
  const body = bodySource.endsWith("\n") ? bodySource : `${bodySource}\n`;
  return body;
}

function hasEsmExport(source: string, name: string): boolean {
  const escaped = name.replace(/[.*+?^${}()|[\]\\]/g, "\\$&");
  const patterns = [
    new RegExp(`\\bexport\\s+(?:async\\s+)?function\\s+${escaped}\\b`),
    new RegExp(`\\bexport\\s+(?:const|let|var)\\s+${escaped}\\b`),
    new RegExp(`\\bexport\\s*\\{[^}]*\\b${escaped}\\b[^}]*\\}`),
  ];
  return patterns.some((pattern) => pattern.test(source));
}

function astPath(type: AstType): TypePath | undefined {
  if (variantOf(type.kind) !== "Path") {
    return undefined;
  }
  return payloadOf(type.kind) as TypePath;
}

function isOptionType(type: AstType | undefined): boolean {
  if (type === undefined) {
    return false;
  }
  const path = astPath(type);
  return path?.segments[path.segments.length - 1] === "Option";
}

function astBoundaryToJavaScript(expression: string, type: AstType): string {
  if (isOptionType(type)) {
    return `(${expression}.$option ? ${expression}.value : null)`;
  }
  return expression;
}

function astBoundaryFromJavaScript(
  expression: string,
  type: AstType | undefined,
  policy: "wrapping" | "safe-checked" | "arbitrary" = "wrapping",
  optionStyle: "null" | "objects" = "null",
): string {
  if (isOptionType(type)) {
    if (optionStyle === "objects" && type !== undefined) {

      const nullStyle = astTypeToTs(type, policy, "null");
      return `((value: ${nullStyle} | undefined) => value === null || value === undefined ? $none : $some(value))(${expression})`;
    }
    return `((value) => value === null || value === undefined ? $none : $some(value))(${expression})`;
  }
  return expression;
}

function scriptErrorTypeName(path: TypePath | undefined): string {
  return path?.segments[path.segments.length - 1] ?? "ScriptError";
}

function createScriptSourceMap(
  code: string,
  sourcePath: string,
  sourceText: string,
  bodySpan: Span,
): string {
  const headerLines = 2;
  const bodyStartLine = lineAt(sourceText, bodySpan.start);
  const sourceLineCount = sourceText.split("\n").length;
  const generatedLines = code.split("\n");
  let previousSourceLine = 0;
  let previousSourceColumn = 0;
  const mappings = generatedLines.map((_, generatedLine) => {
    if (generatedLine < headerLines) {
      return "";
    }
    const sourceLine = bodyStartLine + (generatedLine - headerLines);
    if (sourceLine >= sourceLineCount) {
      return "";
    }
    const segment =
      encodeVlq(0) +
      encodeVlq(0) +
      encodeVlq(sourceLine - previousSourceLine) +
      encodeVlq(0 - previousSourceColumn);
    previousSourceLine = sourceLine;
    previousSourceColumn = 0;
    return segment;
  });

  return JSON.stringify({
    version: 3,
    sources: [sourcePath],
    sourcesContent: [sourceText],
    names: [],
    mappings: mappings.join(";"),
  });
}

function composeTranspiledScriptSourceMap(
  sourcePath: string,
  sourceText: string,
  transpiledMap: string | undefined,
): string | undefined {
  if (transpiledMap === undefined) {
    return undefined;
  }
  let parsed: unknown;
  try {
    parsed = JSON.parse(transpiledMap);
  } catch {
    return undefined;
  }
  if (!isRecord(parsed) || typeof parsed["mappings"] !== "string") {
    return undefined;
  }
  const headerLines = 2;
  const mappings = `${";".repeat(headerLines)}${parsed["mappings"]}`;
  const names = Array.isArray(parsed["names"])
    ? parsed["names"].filter((name): name is string => typeof name === "string")
    : [];
  return JSON.stringify({
    version: 3,
    sources: [sourcePath],
    sourcesContent: [sourceText],
    names,
    mappings,
  });
}

function createSourceMap(
  code: string,
  sourcePath: string,
  sourceText: string,
  anchors: readonly { readonly text: string; readonly span: Span }[],
): string {
  const mappedLines = new Map<number, number>();
  let searchFrom = 0;
  for (const anchor of anchors) {
    const index = code.indexOf(anchor.text, searchFrom);
    if (index < 0) {
      continue;
    }
    searchFrom = index + anchor.text.length;
    const generatedLine = lineAt(code, index);
    if (!mappedLines.has(generatedLine)) {
      mappedLines.set(generatedLine, lineAt(sourceText, anchor.span.start));
    }
  }

  const lines = code.split("\n");
  let previousSource = 0;
  let previousSourceLine = 0;
  let previousSourceColumn = 0;
  const mappings = lines.map((_, generatedLine) => {
    const sourceLine = mappedLines.get(generatedLine);
    if (sourceLine === undefined) {
      return "";
    }
    const segment =
      encodeVlq(0) +
      encodeVlq(0 - previousSource) +
      encodeVlq(sourceLine - previousSourceLine) +
      encodeVlq(0 - previousSourceColumn);
    previousSource = 0;
    previousSourceLine = sourceLine;
    previousSourceColumn = 0;
    return segment;
  });

  return JSON.stringify({
    version: 3,
    sources: [sourcePath],
    sourcesContent: [sourceText],
    names: [],
    mappings: mappings.join(";"),
  });
}

function lineAt(text: string, offset: number): number {
  let line = 0;
  for (let index = 0; index < offset && index < text.length; index += 1) {
    if (text[index] === "\n") {
      line += 1;
    }
  }
  return line;
}

const BASE64_VLQ =
  "ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

function encodeVlq(value: number): string {
  let remaining = value < 0 ? ((-value) << 1) | 1 : value << 1;
  let output = "";
  do {
    let digit = remaining & 31;
    remaining >>>= 5;
    if (remaining > 0) {
      digit |= 32;
    }
    output += BASE64_VLQ[digit] ?? "";
  } while (remaining > 0);
  return output;
}

function containsVariant(value: unknown, variant: string): boolean {
  if (Array.isArray(value)) {
    return value.some((entry) => containsVariant(entry, variant));
  }
  if (!isRecord(value)) {
    return false;
  }
  if (Object.prototype.hasOwnProperty.call(value, variant)) {
    return true;
  }
  return Object.values(value).some((entry) => containsVariant(entry, variant));
}

function isResultReturnType(type: AstType | null | undefined): boolean {
  if (type === null || type === undefined) {
    return false;
  }
  if (variantOf(type.kind) !== "Path") {
    return false;
  }
  const path = payloadOf(type.kind) as TypePath;
  return path.segments[path.segments.length - 1] === "Result";
}

function stripUnderscores(value: string): string {
  return value.replaceAll("_", "");
}

function stripIntegerSuffix(value: string): string {
  const withoutSuffix = value.endsWith("u8") ? value.slice(0, -2) : value;
  return stripUnderscores(withoutSuffix);
}

function jsName(name: string): string {
  if (IDENTIFIER.test(name) && !JS_RESERVED.has(name)) {
    return name;
  }
  return `${name}$`;
}

function exportName(name: string): string {
  return IDENTIFIER.test(name) ? name : JSON.stringify(name);
}

function formatImportBindings(bindings: readonly ImportBinding[]): string {
  return bindings
    .map((binding) =>
      binding.imported === binding.local
        ? jsName(binding.local)
        : `${exportName(binding.imported)} as ${jsName(binding.local)}`,
    )
    .join(", ");
}

function objectKey(name: string): string {
  return IDENTIFIER.test(name) ? name : JSON.stringify(name);
}

function memberAccess(name: string): string {
  return IDENTIFIER.test(name) ? `.${name}` : `[${JSON.stringify(name)}]`;
}

function helperName(name: HelperName): string {
  return `$${name}`;
}

function indent(text: string): string {
  return text
    .split("\n")
    .map((line) => (line.length === 0 ? line : `  ${line}`))
    .join("\n");
}
