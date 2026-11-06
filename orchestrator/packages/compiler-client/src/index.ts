import { spawnSync } from "node:child_process";
import { statSync } from "node:fs";
import { delimiter, dirname, isAbsolute, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";

export const COMPILER_API_VERSION = "1.0";

export type Mode = "core" | "app" | "web" | "system";

export interface SourceUnit {
  readonly path: string;
  readonly text: string;
}

export interface DiagnosticSpan {
  readonly start: number;
  readonly end: number;
}

export interface CompilerDiagnostic {
  readonly code: string;
  readonly severity: "error" | "warning" | "note" | "help";
  readonly message: string;
  readonly span: DiagnosticSpan | null;
  readonly hint: string | null;
}

export interface FormatResponse {
  readonly apiVersion: string;
  readonly source: string | null;
  readonly diagnostics: readonly CompilerDiagnostic[];
}

export interface AstResponse {
  readonly apiVersion: string;
  readonly ok: boolean;
  readonly ast?: unknown;
  readonly diagnostics: readonly CompilerDiagnostic[];
}

export interface ExplainResponse {
  readonly apiVersion: string;
  readonly ok: boolean;
  readonly inference?: unknown;
  readonly explanation?: string;
  readonly diagnostics: readonly CompilerDiagnostic[];
}

export type IntegerPolicy = "wrapping" | "safe-checked" | "arbitrary";

export interface TypeAnnotationIr {
  readonly span: DiagnosticSpan;

  readonly ty: unknown;
}

export interface ModuleIr {

  readonly key: string;

  readonly path: string;

  readonly ast: unknown;

  readonly hir: unknown;

  readonly annotations: readonly TypeAnnotationIr[];

  readonly manualMemoryScopes: readonly DiagnosticSpan[];
  readonly integerPolicy: IntegerPolicy;
}

export interface CompileRequest {
  readonly source: SourceUnit;
  readonly target?: "js" | "ts" | "c" | string;
  readonly mode?: Mode;
  readonly integerPolicy?: IntegerPolicy;
}

export interface CompileResponse {
  readonly apiVersion: string;
  readonly program: { readonly name: string } | null;
  readonly ir?: ModuleIr;
  readonly diagnostics: readonly CompilerDiagnostic[];
  readonly metadata: Readonly<Record<string, unknown>>;
  readonly artifacts: readonly unknown[];
}

export interface FormatRequest {
  readonly source: SourceUnit;
  readonly target?: string;

  readonly mode?: Mode;
}

export interface CompilerClientOptions {

  readonly binaryPath?: string;

  readonly cwd?: string;

  readonly timeoutMs?: number;
}

export type CompilerClientErrorCode = "NS9003" | "NS9004" | "NS9005" | "NS9006";

export class CompilerClientError extends Error {
  readonly code: CompilerClientErrorCode;

  constructor(code: CompilerClientErrorCode, message: string, options?: ErrorOptions) {
    super(message, options);
    this.name = "CompilerClientError";
    this.code = code;
  }
}

const BINARY_NAME = process.platform === "win32" ? "astra-compiler.exe" : "astra-compiler";

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}

function isFile(path: string): boolean {
  try {
    return statSync(path).isFile();
  } catch {
    return false;
  }
}

function binaryNames(name: string): readonly string[] {
  if (process.platform === "win32" && !name.toLowerCase().endsWith(".exe")) {
    return [name, `${name}.exe`];
  }

  return [name];
}

function resolveBinaryCandidate(value: string, cwd: string): string | undefined {
  const candidate = isAbsolute(value) ? value : resolve(cwd, value);

  for (const name of binaryNames(candidate)) {
    if (isFile(name)) {
      return name;
    }
  }

  if (!value.includes("/") && !value.includes("\\")) {
    const pathValue = process.env["PATH"];
    if (pathValue !== undefined) {
      for (const directory of pathValue.split(delimiter)) {
        if (directory.length === 0) {
          continue;
        }

        for (const name of binaryNames(value)) {
          const path = join(directory, name);
          if (isFile(path)) {
            return path;
          }
        }
      }
    }
  }

  return undefined;
}

export function findCompilerBinary(
  options: Pick<CompilerClientOptions, "binaryPath" | "cwd"> = {},
): string {
  const cwd = resolve(options.cwd ?? process.cwd());
  const explicit = options.binaryPath ?? process.env["ASTRA_COMPILER_BIN"];

  if (explicit !== undefined && explicit.length > 0) {
    const found = resolveBinaryCandidate(explicit, cwd);
    if (found !== undefined) {
      return found;
    }

    throw new CompilerClientError(
      "NS9003",
      `Compiler binary '${explicit}' was not found. Set ASTRA_COMPILER_BIN or run "npm run bootstrap".`,
    );
  }

  const roots = new Set<string>();
  const addAncestors = (start: string): void => {
    let current = resolve(start);

    for (;;) {
      roots.add(current);
      const parent = dirname(current);
      if (parent === current) {
        return;
      }
      current = parent;
    }
  };

  addAncestors(cwd);

  const entry = process.argv[1];
  const entryDir = entry === undefined ? undefined : dirname(resolve(entry));
  const packagedCandidates = [
    ...(entryDir === undefined
      ? []
      : [join(entryDir, BINARY_NAME), join(entryDir, "..", "bin", BINARY_NAME)]),
    join(dirname(process.execPath), BINARY_NAME),
  ];
  for (const candidate of packagedCandidates) {
    if (isFile(candidate)) {
      return candidate;
    }
  }

  try {
    addAncestors(fileURLToPath(new URL("../../../../", import.meta.url)));
  } catch {

  }

  for (const root of roots) {
    for (const profile of ["debug", "release"]) {
      const candidate = join(root, "target", profile, BINARY_NAME);
      if (isFile(candidate)) {
        return candidate;
      }
    }
  }

  throw new CompilerClientError(
    "NS9003",
    `Compiler binary '${BINARY_NAME}' was not found. Build it with "cargo build -p astra-compiler-api --bin astra-compiler" or set ASTRA_COMPILER_BIN.`,
  );
}

function parseDiagnostic(value: unknown): CompilerDiagnostic {
  if (!isRecord(value)) {
    throw new CompilerClientError("NS9005", "Compiler returned an invalid diagnostic.");
  }

  const { code, severity, message } = value;
  const spanValue = value["span"] ?? null;
  const hintValue = value["hint"] ?? null;

  if (
    typeof code !== "string" ||
    (severity !== "error" &&
      severity !== "warning" &&
      severity !== "note" &&
      severity !== "help") ||
    typeof message !== "string" ||
    (hintValue !== null && typeof hintValue !== "string")
  ) {
    throw new CompilerClientError("NS9005", "Compiler returned an invalid diagnostic.");
  }

  let span: DiagnosticSpan | null = null;
  if (spanValue !== null) {
    if (
      !isRecord(spanValue) ||
      typeof spanValue["start"] !== "number" ||
      typeof spanValue["end"] !== "number"
    ) {
      throw new CompilerClientError("NS9005", "Compiler returned an invalid diagnostic span.");
    }

    span = { start: spanValue["start"], end: spanValue["end"] };
  }

  return {
    code,
    severity,
    message,
    span,
    hint: hintValue,
  };
}

function parseFormatResponse(value: unknown): FormatResponse {
  if (!isRecord(value)) {
    throw new CompilerClientError("NS9005", "Compiler returned an invalid format result.");
  }

  const { apiVersion, source, diagnostics } = value;
  if (typeof apiVersion !== "string" || apiVersion !== COMPILER_API_VERSION) {
    throw new CompilerClientError(
      "NS9005",
      `Compiler returned an incompatible format result; expected API ${COMPILER_API_VERSION}.`,
    );
  }

  if ((source !== null && typeof source !== "string") || !Array.isArray(diagnostics)) {
    throw new CompilerClientError("NS9005", "Compiler returned an invalid format result.");
  }

  return {
    apiVersion,
    source,
    diagnostics: diagnostics.map(parseDiagnostic),
  };
}

function parseModuleIr(value: unknown): ModuleIr {
  if (!isRecord(value)) {
    throw new CompilerClientError("NS9005", "Compiler returned an invalid typed module.");
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
    throw new CompilerClientError("NS9005", "Compiler returned an invalid typed module.");
  }

  const parsedScopes = manualMemoryScopes.map((scope): DiagnosticSpan => {
    if (!isRecord(scope) || typeof scope["start"] !== "number" || typeof scope["end"] !== "number") {
      throw new CompilerClientError(
        "NS9005",
        "Compiler returned an invalid manual-memory scope.",
      );
    }
    return { start: scope["start"], end: scope["end"] };
  });

  return {
    key,
    path,
    ast,
    hir,
    annotations: annotations.map((annotation): TypeAnnotationIr => {
      if (!isRecord(annotation) || !isRecord(annotation["span"])) {
        throw new CompilerClientError(
          "NS9005",
          "Compiler returned an invalid type annotation.",
        );
      }
      const span = annotation["span"];
      if (typeof span["start"] !== "number" || typeof span["end"] !== "number") {
        throw new CompilerClientError(
          "NS9005",
          "Compiler returned an invalid type annotation span.",
        );
      }
      return {
        span: { start: span["start"], end: span["end"] },
        ty: annotation["ty"] as TypeAnnotationIr["ty"],
      };
    }),
    manualMemoryScopes: parsedScopes,
    integerPolicy,
  };
}

function parseCompileResponse(value: unknown): CompileResponse {
  if (!isRecord(value)) {
    throw new CompilerClientError("NS9005", "Compiler returned an invalid compile result.");
  }

  const { apiVersion, program, diagnostics, metadata, artifacts } = value;
  const ir = value["ir"];
  if (
    typeof apiVersion !== "string" ||
    apiVersion !== COMPILER_API_VERSION ||
    !Array.isArray(diagnostics) ||
    !isRecord(metadata) ||
    !Array.isArray(artifacts)
  ) {
    throw new CompilerClientError(
      "NS9005",
      `Compiler returned an incompatible compile result; expected API ${COMPILER_API_VERSION}.`,
    );
  }

  const parsedProgram =
    program === null || program === undefined
      ? null
      : isRecord(program) && typeof program["name"] === "string"
        ? { name: program["name"] }
        : null;

  return {
    apiVersion,
    program: parsedProgram,
    ...(ir === undefined ? {} : { ir: parseModuleIr(ir) }),
    diagnostics: diagnostics.map(parseDiagnostic),
    metadata,
    artifacts,
  };
}

export class CompilerClient {
  readonly binaryPath: string;

  private readonly cwd: string;
  private readonly timeoutMs: number | undefined;

  constructor(options: CompilerClientOptions = {}) {
    this.binaryPath = findCompilerBinary(options);
    this.cwd = resolve(options.cwd ?? process.cwd());
    this.timeoutMs = options.timeoutMs;
  }

  compile(request: CompileRequest): CompileResponse {
    const payload = JSON.stringify({
      jsonrpc: "2.0",
      id: "astra-compile",
      method: "compile",
      params: {
        apiVersion: COMPILER_API_VERSION,
        source: request.source,
        target: request.target ?? "js",
        ...(request.mode === undefined ? {} : { mode: request.mode }),
        ...(request.integerPolicy === undefined
          ? {}
          : { integerPolicy: request.integerPolicy }),
      },
    });

    const result = spawnSync(this.binaryPath, [], {
      input: payload,
      cwd: this.cwd,
      encoding: "utf8",
      maxBuffer: 32 * 1024 * 1024,
      windowsHide: true,
      ...(this.timeoutMs === undefined ? {} : { timeout: this.timeoutMs }),
    });

    if (result.error !== undefined) {
      throw new CompilerClientError(
        "NS9004",
        `Unable to run compiler '${this.binaryPath}': ${result.error.message}`,
        { cause: result.error },
      );
    }

    if (result.status !== 0) {
      const detail = result.stderr.trim();
      throw new CompilerClientError(
        "NS9004",
        `Compiler exited with status ${result.status ?? "unknown"}${detail.length === 0 ? "" : `: ${detail}`}`,
      );
    }

    const output = result.stdout.trim();
    if (output.length === 0) {
      throw new CompilerClientError("NS9005", "Compiler returned an empty response.");
    }

    let response: unknown;
    try {
      response = JSON.parse(output);
    } catch (error) {
      throw new CompilerClientError("NS9005", "Compiler returned invalid JSON.", {
        cause: error,
      });
    }

    if (!isRecord(response) || response["jsonrpc"] !== "2.0") {
      throw new CompilerClientError("NS9005", "Compiler returned an invalid JSON-RPC response.");
    }

    if (response["error"] !== undefined && response["error"] !== null) {
      const rpcError = response["error"];
      const code =
        isRecord(rpcError) && typeof rpcError["code"] === "number"
          ? rpcError["code"]
          : -32603;
      const message =
        isRecord(rpcError) && typeof rpcError["message"] === "string"
          ? rpcError["message"]
          : "Unknown compiler error.";
      throw new CompilerClientError("NS9006", `Compiler request failed (${code}): ${message}`);
    }

    return parseCompileResponse(response["result"]);
  }

  format(request: FormatRequest): FormatResponse {
    const payload = JSON.stringify({
      jsonrpc: "2.0",
      id: "astra-format",
      method: "format",
      params: {
        apiVersion: COMPILER_API_VERSION,
        source: request.source,
        target: request.target ?? "js",
        ...(request.mode === undefined ? {} : { mode: request.mode }),
      },
    });

    const result = spawnSync(this.binaryPath, [], {
      input: payload,
      cwd: this.cwd,
      encoding: "utf8",
      maxBuffer: 16 * 1024 * 1024,
      windowsHide: true,
      ...(this.timeoutMs === undefined ? {} : { timeout: this.timeoutMs }),
    });

    if (result.error !== undefined) {
      throw new CompilerClientError(
        "NS9004",
        `Unable to run compiler '${this.binaryPath}': ${result.error.message}`,
        { cause: result.error },
      );
    }

    if (result.status !== 0) {
      const detail = result.stderr.trim();
      throw new CompilerClientError(
        "NS9004",
        `Compiler exited with status ${result.status ?? "unknown"}${detail.length === 0 ? "" : `: ${detail}`}`,
      );
    }

    const output = result.stdout.trim();
    if (output.length === 0) {
      throw new CompilerClientError("NS9005", "Compiler returned an empty response.");
    }

    let response: unknown;
    try {
      response = JSON.parse(output);
    } catch (error) {
      throw new CompilerClientError("NS9005", "Compiler returned invalid JSON.", { cause: error });
    }

    if (!isRecord(response) || response["jsonrpc"] !== "2.0") {
      throw new CompilerClientError("NS9005", "Compiler returned an invalid JSON-RPC response.");
    }

    if (response["error"] !== undefined && response["error"] !== null) {
      const rpcError = response["error"];
      const code =
        isRecord(rpcError) && typeof rpcError["code"] === "number"
          ? rpcError["code"]
          : -32603;
      const message =
        isRecord(rpcError) && typeof rpcError["message"] === "string"
          ? rpcError["message"]
          : "Unknown compiler error.";
      throw new CompilerClientError("NS9006", `Compiler request failed (${code}): ${message}`);
    }

    return parseFormatResponse(response["result"]);
  }

  formatSource(
    path: string,
    text: string,
    options: Pick<FormatRequest, "target" | "mode"> = {},
  ): FormatResponse {
    return this.format({ source: { path, text }, ...options });
  }

  ast(request: CompileRequest): AstResponse {
    const result = this.send(
      "ast",
      "astra-ast",
      this.compileParams(request),
      32 * 1024 * 1024,
    );
    if (
      !isRecord(result) ||
      typeof result["apiVersion"] !== "string" ||
      typeof result["ok"] !== "boolean" ||
      !Array.isArray(result["diagnostics"])
    ) {
      throw new CompilerClientError("NS9005", "Compiler returned an invalid AST result.");
    }
    if (result["apiVersion"] !== COMPILER_API_VERSION) {
      throw new CompilerClientError(
        "NS9005",
        `Compiler returned API ${result["apiVersion"]}; expected ${COMPILER_API_VERSION}.`,
      );
    }
    const ast = result["ast"];
    return {
      apiVersion: result["apiVersion"],
      ok: result["ok"],
      ...(ast === undefined ? {} : { ast }),
      diagnostics: result["diagnostics"].map(parseDiagnostic),
    };
  }

  explain(request: CompileRequest): ExplainResponse {
    const result = this.send(
      "explain",
      "astra-explain",
      this.compileParams(request),
      32 * 1024 * 1024,
    );
    if (
      !isRecord(result) ||
      typeof result["apiVersion"] !== "string" ||
      typeof result["ok"] !== "boolean" ||
      !Array.isArray(result["diagnostics"])
    ) {
      throw new CompilerClientError("NS9005", "Compiler returned an invalid explain result.");
    }
    if (result["apiVersion"] !== COMPILER_API_VERSION) {
      throw new CompilerClientError(
        "NS9005",
        `Compiler returned API ${result["apiVersion"]}; expected ${COMPILER_API_VERSION}.`,
      );
    }
    const inference = result["inference"];
    const explanation = result["explanation"];
    return {
      apiVersion: result["apiVersion"],
      ok: result["ok"],
      ...(inference === undefined ? {} : { inference }),
      ...(typeof explanation === "string" ? { explanation } : {}),
      diagnostics: result["diagnostics"].map(parseDiagnostic),
    };
  }

  private compileParams(request: CompileRequest): Record<string, unknown> {
    return {
      apiVersion: COMPILER_API_VERSION,
      source: request.source,
      target: request.target ?? "js",
      ...(request.mode === undefined ? {} : { mode: request.mode }),
      ...(request.integerPolicy === undefined
        ? {}
        : { integerPolicy: request.integerPolicy }),
    };
  }

  private send(
    method: string,
    id: string,
    params: unknown,
    maxBuffer: number,
  ): unknown {
    const payload = JSON.stringify({ jsonrpc: "2.0", id, method, params });
    const result = spawnSync(this.binaryPath, [], {
      input: payload,
      cwd: this.cwd,
      encoding: "utf8",
      maxBuffer,
      windowsHide: true,
      ...(this.timeoutMs === undefined ? {} : { timeout: this.timeoutMs }),
    });

    if (result.error !== undefined) {
      throw new CompilerClientError(
        "NS9004",
        `Unable to run compiler '${this.binaryPath}': ${result.error.message}`,
        { cause: result.error },
      );
    }
    if (result.status !== 0) {
      const detail = result.stderr.trim();
      throw new CompilerClientError(
        "NS9004",
        `Compiler exited with status ${result.status ?? "unknown"}${detail.length === 0 ? "" : `: ${detail}`}`,
      );
    }

    const output = result.stdout.trim();
    if (output.length === 0) {
      throw new CompilerClientError("NS9005", "Compiler returned an empty response.");
    }

    let response: unknown;
    try {
      response = JSON.parse(output);
    } catch (error) {
      throw new CompilerClientError("NS9005", "Compiler returned invalid JSON.", { cause: error });
    }

    if (!isRecord(response) || response["jsonrpc"] !== "2.0") {
      throw new CompilerClientError("NS9005", "Compiler returned an invalid JSON-RPC response.");
    }

    if (response["error"] !== undefined && response["error"] !== null) {
      const rpcError = response["error"];
      const code =
        isRecord(rpcError) && typeof rpcError["code"] === "number" ? rpcError["code"] : -32603;
      const message =
        isRecord(rpcError) && typeof rpcError["message"] === "string"
          ? rpcError["message"]
          : "Unknown compiler error.";
      throw new CompilerClientError("NS9006", `Compiler request failed (${code}): ${message}`);
    }

    return response["result"];
  }
}

export interface ProjectSourceUnit {

  readonly key: string;

  readonly path: string;

  readonly text: string;
}

export interface ModeReason {
  readonly kind: string;
  readonly mode: string;
  readonly detail: string;
  readonly capability?: string;
  readonly module?: string;
  readonly span?: DiagnosticSpan;
}

export interface ProjectCheckRequest {

  readonly sources: readonly ProjectSourceUnit[];
  readonly target?: string;
  readonly mode?: Mode;
  readonly integerPolicy?: IntegerPolicy;
}

export interface ModuleCheckResult {
  readonly key: string;
  readonly path: string;
  readonly diagnostics: readonly CompilerDiagnostic[];
  readonly ok: boolean;

  readonly ir?: ModuleIr;
}

export interface ProjectCheckResponse {
  readonly apiVersion: string;
  readonly ok: boolean;
  readonly modules: readonly ModuleCheckResult[];
  readonly program: { readonly name: string } | null;
  readonly metadata: {
    readonly moduleOrder: readonly string[];
    readonly target: string;

    readonly mode: string;

    readonly modeSource?: "explicit" | "inferred";
    readonly requiredMode?: string;
    readonly inferredModes?: Readonly<Record<string, string>>;
    readonly modeReasons?: readonly ModeReason[];
    readonly modeExplanation?: string;
    [key: string]: unknown;
  };
}

export interface ProjectCompileRequest {

  readonly sources: readonly ProjectSourceUnit[];
  readonly target?: string;
  readonly mode?: Mode;
  readonly integerPolicy?: IntegerPolicy;

  readonly library?: boolean;

  readonly importMap?: Readonly<Record<string, string>>;
}

export interface ModuleCompileArtifact {
  readonly key: string;
  readonly path: string;

  readonly cFilename: string;

  readonly hFilename: string;
  readonly cSource: string;
  readonly hSource: string;

  readonly scriptSources: readonly (readonly [string, string])[];
}

export interface ProjectCompileResponse {
  readonly apiVersion: string;
  readonly ok: boolean;
  readonly modules: readonly ModuleCompileArtifact[];

  readonly hostShim: string;
  readonly diagnostics: readonly CompilerDiagnostic[];
  readonly metadata: Readonly<Record<string, unknown>>;
}

function parseModuleCompileArtifact(value: unknown): ModuleCompileArtifact {
  if (!isRecord(value)) {
    throw new CompilerClientError("NS9005", "Compiler returned an invalid module artifact.");
  }
  const { key, path, cFilename, hFilename, cSource, hSource, scriptSources } = value;
  if (
    typeof key !== "string" ||
    typeof path !== "string" ||
    typeof cFilename !== "string" ||
    typeof hFilename !== "string" ||
    typeof cSource !== "string" ||
    typeof hSource !== "string" ||
    !Array.isArray(scriptSources)
  ) {
    throw new CompilerClientError("NS9005", "Compiler returned an invalid module artifact.");
  }
  return {
    key,
    path,
    cFilename,
    hFilename,
    cSource,
    hSource,
    scriptSources: scriptSources.map((entry) => {
      if (!Array.isArray(entry) || entry.length !== 2) {
        throw new CompilerClientError("NS9005", "Compiler returned an invalid script source.");
      }
      return [String(entry[0]), String(entry[1])] as const;
    }),
  };
}

function parseProjectCompileResponse(value: unknown): ProjectCompileResponse {
  if (!isRecord(value)) {
    throw new CompilerClientError("NS9005", "Compiler returned an invalid project compile result.");
  }
  const { apiVersion, ok, modules, hostShim, diagnostics, metadata } = value;
  if (
    typeof apiVersion !== "string" ||
    apiVersion !== COMPILER_API_VERSION ||
    typeof ok !== "boolean" ||
    !Array.isArray(modules) ||
    typeof hostShim !== "string" ||
    !Array.isArray(diagnostics)
  ) {
    throw new CompilerClientError(
      "NS9005",
      `Compiler returned an incompatible project compile result; expected API ${COMPILER_API_VERSION}.`,
    );
  }
  return {
    apiVersion,
    ok,
    modules: modules.map(parseModuleCompileArtifact),
    hostShim,
    diagnostics: diagnostics.map(parseDiagnostic),
    metadata: isRecord(metadata) ? metadata : {},
  };
}

function parseModuleCheckResult(value: unknown): ModuleCheckResult {
  if (!isRecord(value)) {
    throw new CompilerClientError("NS9005", "Compiler returned an invalid module result.");
  }
  const { key, path, ok } = value;
  const diagnosticsRaw = value["diagnostics"];
  const irRaw = value["ir"];
  if (
    typeof key !== "string" ||
    typeof path !== "string" ||
    typeof ok !== "boolean" ||
    !Array.isArray(diagnosticsRaw)
  ) {
    throw new CompilerClientError("NS9005", "Compiler returned an invalid module result.");
  }
  return {
    key,
    path,
    ok,
    diagnostics: diagnosticsRaw.map(parseDiagnostic),
    ...(irRaw === undefined ? {} : { ir: parseModuleIr(irRaw) }),
  };
}

function parseProjectCheckResponse(value: unknown): ProjectCheckResponse {
  if (!isRecord(value)) {
    throw new CompilerClientError("NS9005", "Compiler returned an invalid project check result.");
  }
  const { apiVersion, ok, modules, program, metadata } = value;
  if (
    typeof apiVersion !== "string" ||
    apiVersion !== COMPILER_API_VERSION ||
    typeof ok !== "boolean" ||
    !Array.isArray(modules)
  ) {
    throw new CompilerClientError(
      "NS9005",
      `Compiler returned an incompatible project check result; expected API ${COMPILER_API_VERSION}.`,
    );
  }

  const parsedProgram =
    program === null || program === undefined
      ? null
      : isRecord(program) && typeof program["name"] === "string"
        ? { name: program["name"] }
        : null;

  const parsedMetadata = isRecord(metadata)
    ? (metadata as { moduleOrder: string[]; target: string; mode: string })
    : { moduleOrder: [], target: "js", mode: "core" };

  return {
    apiVersion,
    ok,
    modules: modules.map(parseModuleCheckResult),
    program: parsedProgram,
    metadata: parsedMetadata,
  };
}

declare module "./index.js" {}

CompilerClient.prototype.checkProject = function (
  this: CompilerClient,
  request: ProjectCheckRequest,
): ProjectCheckResponse {
  const payload = JSON.stringify({
    jsonrpc: "2.0",
    id: "astra-check-project",
    method: "check_project",
    params: {
      apiVersion: COMPILER_API_VERSION,
      sources: request.sources,
      target: request.target ?? "js",
      ...(request.mode === undefined ? {} : { mode: request.mode }),
      ...(request.integerPolicy === undefined
        ? {}
        : { integerPolicy: request.integerPolicy }),
    },
  });

  const result = spawnSync(this.binaryPath, [], {
    input: payload,
    cwd: (this as unknown as { cwd: string }).cwd,
    encoding: "utf8",
    maxBuffer: 16 * 1024 * 1024,
    windowsHide: true,
  });

  if (result.error !== undefined) {
    throw new CompilerClientError(
      "NS9004",
      `Unable to run compiler '${this.binaryPath}': ${result.error.message}`,
      { cause: result.error },
    );
  }
  if (result.status !== 0) {
    const detail = result.stderr.trim();
    throw new CompilerClientError(
      "NS9004",
      `Compiler exited with status ${result.status ?? "unknown"}${detail.length === 0 ? "" : `: ${detail}`}`,
    );
  }

  const output = result.stdout.trim();
  if (output.length === 0) {
    throw new CompilerClientError("NS9005", "Compiler returned an empty response.");
  }

  let response: unknown;
  try {
    response = JSON.parse(output);
  } catch (error) {
    throw new CompilerClientError("NS9005", "Compiler returned invalid JSON.", { cause: error });
  }

  if (!isRecord(response) || response["jsonrpc"] !== "2.0") {
    throw new CompilerClientError("NS9005", "Compiler returned an invalid JSON-RPC response.");
  }

  if (response["error"] !== undefined && response["error"] !== null) {
    const rpcError = response["error"];
    const code =
      isRecord(rpcError) && typeof rpcError["code"] === "number" ? rpcError["code"] : -32603;
    const message =
      isRecord(rpcError) && typeof rpcError["message"] === "string"
        ? rpcError["message"]
        : "Unknown compiler error.";
    throw new CompilerClientError("NS9006", `Compiler request failed (${code}): ${message}`);
  }

  return parseProjectCheckResponse(response["result"]);
};

export interface CompilerClient {

  checkProject(request: ProjectCheckRequest): ProjectCheckResponse;
}

CompilerClient.prototype.compileProject = function (
  this: CompilerClient,
  request: ProjectCompileRequest,
): ProjectCompileResponse {
  const payload = JSON.stringify({
    jsonrpc: "2.0",
    id: "astra-compile-project",
    method: "compile_project",
    params: {
      apiVersion: COMPILER_API_VERSION,
      sources: request.sources,
      target: request.target ?? "c",
      ...(request.mode === undefined ? {} : { mode: request.mode }),
      ...(request.integerPolicy === undefined
        ? {}
        : { integerPolicy: request.integerPolicy }),
      ...(request.library === true ? { library: true } : {}),
      ...(request.importMap === undefined || Object.keys(request.importMap).length === 0
        ? {}
        : { importMap: request.importMap }),
    },
  });

  const result = spawnSync(this.binaryPath, [], {
    input: payload,
    cwd: (this as unknown as { cwd: string }).cwd,
    encoding: "utf8",
    maxBuffer: 64 * 1024 * 1024,
    windowsHide: true,
  });

  if (result.error !== undefined) {
    throw new CompilerClientError(
      "NS9004",
      `Unable to run compiler '${this.binaryPath}': ${result.error.message}`,
      { cause: result.error },
    );
  }
  if (result.status !== 0) {
    const detail = result.stderr.trim();
    throw new CompilerClientError(
      "NS9004",
      `Compiler exited with status ${result.status ?? "unknown"}${detail.length === 0 ? "" : `: ${detail}`}`,
    );
  }

  const output = result.stdout.trim();
  if (output.length === 0) {
    throw new CompilerClientError("NS9005", "Compiler returned an empty response.");
  }

  let response: unknown;
  try {
    response = JSON.parse(output);
  } catch (error) {
    throw new CompilerClientError("NS9005", "Compiler returned invalid JSON.", { cause: error });
  }

  if (!isRecord(response) || response["jsonrpc"] !== "2.0") {
    throw new CompilerClientError("NS9005", "Compiler returned an invalid JSON-RPC response.");
  }

  if (response["error"] !== undefined && response["error"] !== null) {
    const rpcError = response["error"];
    const code =
      isRecord(rpcError) && typeof rpcError["code"] === "number" ? rpcError["code"] : -32603;
    const message =
      isRecord(rpcError) && typeof rpcError["message"] === "string"
        ? rpcError["message"]
        : "Unknown compiler error.";
    throw new CompilerClientError("NS9006", `Compiler request failed (${code}): ${message}`);
  }

  return parseProjectCompileResponse(response["result"]);
};

export interface CompilerClient {

  compileProject(request: ProjectCompileRequest): ProjectCompileResponse;
}
