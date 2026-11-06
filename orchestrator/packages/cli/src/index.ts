#!/usr/bin/env node

import { spawnSync } from "node:child_process";
import { existsSync, mkdirSync, readFileSync, readdirSync, rmSync, statSync, writeFileSync } from "node:fs";
import {
  basename,
  dirname,
  extname,
  isAbsolute,
  join,
  relative,
  resolve,
  sep,
} from "node:path";
import { TextDecoder } from "node:util";
import { pathToFileURL } from "node:url";
import { generateJavaScript } from "@neo-astra/codegen-js";
import { generateTypeScript } from "@neo-astra/codegen-ts";
import {
  CompilerClient,
  CompilerClientError,
  type AstResponse,
  type CompilerDiagnostic,
  type ExplainResponse,
  type Mode,
} from "@neo-astra/compiler-client";
import {
  ManifestError,
  formatManifestSource,
  getPackageName,
  loadManifest,
  loadBuildManifest,
  loadProjectManifests,
  applyBuildDefaults,
  manifestFileForPath,
  type ManifestDocument,
} from "@neo-astra/manifest";
import { parseToolchainPolicy, resolveToolchainAsync, doctorAsync, ToolchainError, compilerInvocation, type ToolchainPolicy, type ToolchainProbe } from "@neo-astra/toolchain";

const VERSION = "1.0.0";
const SOURCE_EXTENSION = ".ns";

const IGNORED_DIRECTORIES = new Set([
  "build",
  "coverage",
  "dist",
  "node_modules",
  "out",
  "target",
  ".astra",
  ".turbo",
  ".next",
]);

const HELP = `Neo Astra compiler

Usage:
  astra <command> [options]

Commands:
  check [dir] [--json]   Discover, validate, and check the project
  doctor [dir] [--json]  Resolve and print the toolchain ladder
  build [dir]            Build the configured target
  fmt [path] [--check]   Format .ns source and core.json/core.toml
  ast [path]             Print the parsed AST of a file or the project entry
  explain [path]         Explain mode/capability inference for a file or entry

Environment:
  ASTRA_CC               Override the C compiler (path or command)

Options:
  -h, --help             Show this help
  -v, --version          Show the CLI version

build options:
  --target <name>        Select a target from the manifest
  --emit <kind>          Artifact kind: executable, staticlib, or cdylib
  --compiler <path>      Use a specific astra-compiler binary
  --json                 Print a machine-readable build report
  --no-download          Do not fetch a missing pinned C toolchain

doctor options:
  --json                 Print a machine-readable diagnosis
  --no-download          Do not fetch a missing pinned C toolchain

fmt options:
  --check                Report formatting differences without writing
  --json                 Print a machine-readable report
  --compiler <path>      Use a specific astra-compiler binary

ast/explain options:
  --json                 Print the full compiler response
  --compiler <path>      Use a specific astra-compiler binary
  --mode <mode>          core, app, web, or system (default: inferred)
  --target <name>        Compiler target hint (default: js)

Exit codes:
  0  success
  1  invalid input, project error, or build/codegen failure
`;

type CliErrorCode =
  | "NS8001"
  | "NS8002"
  | "NS8003"
  | "NS8004"
  | "NS9000"
  | "NS9001"
  | "NS9007"
  | "NS9008"
  | "NS9009"
  | "NS9010"
  | "NS9011"
  | "NS9012"
  | "NS9013"
  | "NS9014"
  | "NS9015"
  | "NS9016"
  | "NS9101"
  | "NS9102"
  | "NS9103"
  | "NV0002";

type FormatFileKind = "source" | "manifest";

interface SourceFile {
  readonly path: string;
  readonly displayPath: string;
  readonly kind: FormatFileKind;
}

interface ReportedDiagnostic {
  readonly code: string;
  readonly severity: CompilerDiagnostic["severity"];
  readonly message: string;
  readonly hint: string | null;
  readonly path: string | null;
  readonly line: number | null;
  readonly column: number | null;
}

type FileStatus = "formatted" | "unchanged" | "would-reformat" | "error";

interface FileResult {
  readonly file: SourceFile;
  readonly status: FileStatus;
  readonly formatted: string | null;
  readonly diagnostics: readonly ReportedDiagnostic[];
}

interface FmtOptions {
  readonly path: string;
  readonly check: boolean;
  readonly json: boolean;
  readonly compilerPath: string | undefined;
}

interface BuildOptions {
  readonly project: string;
  readonly target: string | undefined;
  readonly emit: string | undefined;
  readonly json: boolean;
  readonly compilerPath: string | undefined;
  readonly noDownload: boolean;
}

interface InspectionOptions {
  readonly input: string;
  readonly json: boolean;
  readonly compilerPath: string | undefined;
  readonly mode: Mode | undefined;
  readonly target: string | undefined;
}

interface BuildReport {
  readonly target: string;
  readonly backend: string;
  readonly entry: string;
  readonly output: string | null;
  readonly diagnostics: readonly ReportedDiagnostic[];
}

class CliError extends Error {
  readonly code: CliErrorCode;
  readonly path: string | undefined;

  constructor(code: CliErrorCode, message: string, path?: string) {
    super(message);
    this.name = "CliError";
    this.code = code;
    this.path = path;
  }
}

function printHelp(): void {
  process.stdout.write(HELP);
}

function printVersion(): void {
  process.stdout.write(`astra ${VERSION}\n`);
}

function manifestJson(manifest: ManifestDocument): string {
  return JSON.stringify(
    {
      manifest: {
        path: manifest.file.path,
        name: manifest.file.name,
        format: manifest.file.format,
      },
      package: getPackageName(manifest) ?? null,
    },
    null,
    2,
  );
}

function manifestText(manifest: ManifestDocument): string {
  const packageName = getPackageName(manifest);
  const lines = [
    `Manifest: ${manifest.file.path}`,
    `Format: ${manifest.file.format}`,
  ];

  if (packageName !== undefined) {
    lines.push(`Package: ${packageName}`);
  }

  return `${lines.join("\n")}\n`;
}

function diagnosticJson(error: ManifestError): string {
  return JSON.stringify(
    {
      code: error.code,
      severity: "error",
      message: error.message,
      ...(error.path === undefined ? {} : { path: error.path }),
    },
    null,
    2,
  );
}

function isJsonFlag(args: readonly string[]): boolean {
  return args.includes("--json");
}

function commandDirectory(args: readonly string[], cwd: string): string {
  const candidate = args.find((arg) => !arg.startsWith("-"));
  return candidate ?? cwd;
}

interface FfiDeclaration {
  readonly kind: "foreign" | "script";
  readonly name: string;
  readonly span: CompilerDiagnostic["span"];
}

function collectFfiDeclarations(ast: unknown): readonly FfiDeclaration[] {
  if (!isRecord(ast) || !Array.isArray(ast["items"])) {
    return [];
  }

  const declarations: FfiDeclaration[] = [];
  const visit = (items: readonly unknown[]): void => {
    for (const item of items) {
      if (!isRecord(item) || !isRecord(item["kind"])) {
        continue;
      }
      const kind = item["kind"];
      const foreign = kind["ForeignModule"];
      if (isRecord(foreign) && typeof foreign["name"] === "string") {
        declarations.push({
          kind: "foreign",
          name: foreign["name"],
          span: isRecord(foreign["span"])
            ? {
                start: Number(foreign["span"]["start"]),
                end: Number(foreign["span"]["end"]),
              }
            : null,
        });
        continue;
      }
      const script = kind["ScriptBlock"];
      if (isRecord(script) && typeof script["name"] === "string") {
        declarations.push({
          kind: "script",
          name: script["name"],
          span: isRecord(script["span"])
            ? {
                start: Number(script["span"]["start"]),
                end: Number(script["span"]["end"]),
              }
            : null,
        });
        continue;
      }
      const scoped = kind["ScopedItems"];
      if (isRecord(scoped) && Array.isArray(scoped["items"])) {
        visit(scoped["items"]);
      }
    }
  };

  visit(ast["items"]);
  return declarations;
}

function ffiMappingProblems(
  name: string,
  mapping: unknown,
  required: boolean,
  activeTarget: string,
): readonly CompilerDiagnostic[] {
  if (mapping === undefined) {
    return required
      ? [
          {
            code: "NS7007",
            severity: "error",
            message: `Foreign module '${name}' has no manifest ffi mapping.`,
            span: null,
            hint: "Add an ffi entry for the module and its active target.",
          },
        ]
      : [];
  }

  if (!isRecord(mapping)) {
    return [
      {
        code: "NS7007",
        severity: "error",
        message: `The manifest ffi mapping for '${name}' must be an object.`,
        span: null,
        hint: "Map the module to one or more target configuration objects.",
      },
    ];
  }

  const targets = Object.entries(mapping);
  if (targets.length === 0) {
    return [
      {
        code: "NS7007",
        severity: "error",
        message: `The manifest ffi mapping for '${name}' is empty.`,
        span: null,
        hint: "Add at least one target mapping.",
      },
    ];
  }

  if (required && !Object.hasOwn(mapping, activeTarget)) {
    return [
      {
        code: "NS7007",
        severity: "error",
        message: `Foreign module '${name}' has no manifest ffi mapping for active target '${activeTarget}'.`,
        span: null,
        hint: `Add an ffi entry for '${activeTarget}' or select a target with a mapping.`,
      },
    ];
  }

  const problems: CompilerDiagnostic[] = [];
  for (const [target, targetMapping] of targets) {
    if (!isRecord(targetMapping)) {
      problems.push({
        code: "NS7007",
        severity: "error",
        message: `The manifest ffi mapping for '${name}' target '${target}' must be an object.`,
        span: null,
        hint: "Provide source and type fields for the target.",
      });
      continue;
    }
    const source = targetMapping["source"];
    const type = targetMapping["type"];
    if (typeof source !== "string" || source.length === 0) {
      problems.push({
        code: "NS7007",
        severity: "error",
        message: `The manifest ffi mapping for '${name}' target '${target}' needs a non-empty source.`,
        span: null,
        hint: "Set source to a path, package specifier, or 'inline'.",
      });
    }
    if (typeof type !== "string" || type.length === 0) {
      problems.push({
        code: "NS7007",
        severity: "error",
        message: `The manifest ffi mapping for '${name}' target '${target}' needs a non-empty type.`,
        span: null,
        hint: "Set type to a supported FFI mapping type.",
      });
    } else if (type.startsWith("inline-") && source !== "inline") {
      problems.push({
        code: "NS7007",
        severity: "error",
        message: `The manifest ffi mapping for '${name}' target '${target}' uses '${type}' but its source is not 'inline'.`,
        span: null,
        hint: "Inline mappings use source: \"inline\".",
      });
    }
    for (const field of ["ownership", "threading", "errors"] as const) {
      const value = targetMapping[field];
      if (typeof value !== "string" || value.length === 0) {
        problems.push({
          code: "NS7007",
          severity: "error",
          message: `The manifest ffi mapping for '${name}' target '${target}' needs a non-empty ${field}.`,
          span: null,
          hint: "State ownership, threading, and error behavior for every FFI mapping.",
        });
      }
    }
  }
  return problems;
}

function sourceCheckTarget(buildData: Record<string, unknown>): string {
  const target = typeof buildData["defaultTarget"] === "string"
    ? buildData["defaultTarget"]
    : "js";
  if (target === "native" || target === "c") {
    return "c";
  }
  if (target === "ts") {
    return "ts";
  }
  return "js";
}

function checkSourceProject(
  root: string,
  entryValue: string,
  cwd: string,
  manifest: ManifestDocument,
  buildData: Record<string, unknown>,
  diagnostics: ReportedDiagnostic[],
): void {
  const entryPath = resolve(root, entryValue);
  if (!existsSync(entryPath) || extname(entryPath).toLowerCase() !== SOURCE_EXTENSION) {
    return;
  }

  const entryKey = relative(root, entryPath).split(sep).join("/");
  const modules = discoverProjectModules(root, entryPath, entryKey, cwd);
  if (modules.length === 0) {
    return;
  }

  const client = new CompilerClient({ cwd: root });
  const target = sourceCheckTarget(buildData);
  const activeTarget =
    typeof buildData["defaultTarget"] === "string"
      ? buildData["defaultTarget"]
      : "native";
  const modeValue = buildData["defaultMode"];
  const mode = isMode(modeValue) ? modeValue : undefined;
  const math = isRecord(buildData["math"]) ? buildData["math"] : {};
  const policyValue = math["integers"];
  const integerPolicy =
    policyValue === "wrapping" ||
    policyValue === "safe-checked" ||
    policyValue === "arbitrary"
      ? policyValue
      : undefined;

  const byKey = new Map(modules.map((module) => [module.key, module]));
  const checked = client.checkProject({
    sources: modules.map((module) => ({
      key: module.key,
      path: relative(root, module.absolutePath).split(sep).join("/"),
      text: module.text,
    })),
    target,
    ...(mode === undefined ? {} : { mode }),
    ...(integerPolicy === undefined ? {} : { integerPolicy }),
  });

  for (const module of checked.modules) {
    const sourceModule = byKey.get(module.key);
    if (sourceModule === undefined) {
      continue;
    }
    const file: SourceFile = {
      path: sourceModule.absolutePath,
      displayPath: displayPath(sourceModule.absolutePath, cwd),
      kind: "source",
    };
    for (const diagnostic of module.diagnostics) {
      diagnostics.push(reportDiagnostic(file, sourceModule.text, diagnostic));
    }
  }

  const ffi = isRecord(manifest.data["ffi"]) ? manifest.data["ffi"] : undefined;
  if (manifest.data["ffi"] !== undefined && ffi === undefined) {
    diagnostics.push({
      code: "NS7007",
      severity: "error",
      message: "The manifest 'ffi' field must be an object.",
      hint: "Map each foreign module name to its target configuration.",
      path: displayPath(manifest.file.path, cwd),
      line: null,
      column: null,
    });
    return;
  }

  for (const module of modules) {
    const file: SourceFile = {
      path: module.absolutePath,
      displayPath: displayPath(module.absolutePath, cwd),
      kind: "source",
    };
    const astResponse = client.ast({
      source: {
        path: relative(root, module.absolutePath).split(sep).join("/"),
        text: module.text,
      },
      target,
    });
    if (!astResponse.ok || astResponse.ast === undefined) {
      continue;
    }
    for (const declaration of collectFfiDeclarations(astResponse.ast)) {
      const mapping = ffi?.[declaration.name];
      const problems = ffiMappingProblems(
        declaration.name,
        mapping,
        declaration.kind === "foreign",
        activeTarget,
      );
      for (const problem of problems) {
        diagnostics.push(
          reportDiagnostic(file, module.text, {
            ...problem,
            span: problem.span ?? declaration.span,
          }),
        );
      }
    }
  }
}

function runCheck(args: readonly string[], cwd: string): number {
  const json = isJsonFlag(args);

  try {
    const manifests = loadProjectManifests(commandDirectory(args, cwd));
    const manifest = manifests.core;
    const root = dirname(manifest.file.path);
    const buildData = applyBuildDefaults(manifests.build?.data);
    const diagnostics: ReportedDiagnostic[] = [];
    if (manifest.data["ffi"] !== undefined && !isRecord(manifest.data["ffi"])) {
      diagnostics.push({
        code: "NS7007",
        severity: "error",
        message: "The manifest 'ffi' field must be an object.",
        hint: "Map each foreign module name to its target configuration.",
        path: displayPath(manifest.file.path, cwd),
        line: null,
        column: null,
      });
    }

    const nova = isRecord(buildData["nova"]) ? buildData["nova"] : {};
    const entryValue =
      typeof nova["entry"] === "string" && nova["entry"].length > 0
        ? nova["entry"]
        : "src/main.ns";

    try {
      checkSourceProject(root, entryValue, cwd, manifest, buildData, diagnostics);
    } catch (error) {
      return emitFatal(error, false, json);
    }

    const hasErrors = diagnostics.some((diagnostic) => diagnostic.severity === "error");
    if (json) {
      const base = JSON.parse(manifestJson(manifest)) as Record<string, unknown>;
      process.stdout.write(
        `${JSON.stringify({ ...base, ok: !hasErrors, diagnostics }, null, 2)}\n`,
      );
    } else {
      process.stdout.write(manifestText(manifest));
      for (const diagnostic of diagnostics) {
        printDiagnostic(diagnostic);
      }
    }
    return hasErrors ? 1 : 0;
  } catch (error) {
    if (error instanceof ManifestError) {
      process.stderr.write(json ? `${diagnosticJson(error)}\n` : `${error.code}: ${error.message}\n`);
      return 1;
    }

    const message = error instanceof Error ? error.message : String(error);
    process.stderr.write(`NS9000: ${message}\n`);
    return 1;
  }
}

function parseInspectionArgs(args: readonly string[], cwd: string): InspectionOptions {
  let json = false;
  let compilerPath: string | undefined;
  let mode: Mode | undefined;
  let target: string | undefined;
  const positional: string[] = [];

  for (let index = 0; index < args.length; index += 1) {
    const arg = args[index];
    if (arg === undefined) {
      continue;
    }
    if (arg === "--json") {
      json = true;
      continue;
    }
    if (arg === "--compiler" || arg === "--mode" || arg === "--target") {
      const value = args[index + 1];
      if (value === undefined || value.startsWith("-")) {
        throw new CliError("NS9000", `The ${arg} option requires a value.`);
      }
      if (arg === "--compiler") {
        compilerPath = value;
      } else if (arg === "--target") {
        target = value;
      } else {
        if (!isMode(value)) {
          throw new CliError(
            "NS9000",
            `Unknown mode '${value}'. Use core, app, web, or system.`,
          );
        }
        mode = value;
      }
      index += 1;
      continue;
    }
    if (arg.startsWith("-")) {
      throw new CliError("NS9000", `Unknown option '${arg}'.`);
    }
    positional.push(arg);
  }

  if (positional.length > 1) {
    throw new CliError("NS9000", "Expected at most one file or directory path.");
  }

  return {
    input: positional[0] ?? cwd,
    json,
    compilerPath,
    mode,
    target,
  };
}

function resolveInspectionSource(
  input: string,
  cwd: string,
): { readonly root: string; readonly file: SourceFile; readonly source: string } {
  const absolute = resolve(cwd, input);
  let stat;
  try {
    stat = statSync(absolute);
  } catch (error) {
    throw new CliError(
      "NS9008",
      `Unable to access '${input}': ${errorMessage(error)}`,
      displayPath(absolute, cwd),
    );
  }

  if (stat.isDirectory()) {
    const manifests = loadProjectManifests(absolute);
    const root = dirname(manifests.core.file.path);
    const buildData = applyBuildDefaults(manifests.build?.data);
    const nova = isRecord(buildData["nova"]) ? buildData["nova"] : {};
    const entry =
      typeof nova["entry"] === "string" && nova["entry"].length > 0
        ? nova["entry"]
        : "src/main.ns";
    const path = resolve(root, entry);
    const file: SourceFile = {
      path,
      displayPath: displayPath(path, cwd),
      kind: "source",
    };
    return { root, file, source: readSource(file) };
  }

  if (!stat.isFile()) {
    throw new CliError(
      "NS9008",
      "The path must be a .ns file or a project directory.",
      displayPath(absolute, cwd),
    );
  }
  if (extname(absolute).toLowerCase() !== SOURCE_EXTENSION) {
    throw new CliError(
      "NS9008",
      "The path must be a .ns file or a project directory.",
      displayPath(absolute, cwd),
    );
  }

  const file: SourceFile = {
    path: absolute,
    displayPath: displayPath(absolute, cwd),
    kind: "source",
  };
  return { root: dirname(absolute), file, source: readSource(file) };
}

function runInspection(command: "ast" | "explain", args: readonly string[], cwd: string): number {
  if (args.includes("-h") || args.includes("--help")) {
    printHelp();
    return 0;
  }

  let options: InspectionOptions;
  try {
    options = parseInspectionArgs(args, cwd);
  } catch (error) {
    return emitFatal(error, false, isJsonFlag(args));
  }

  let resolved;
  try {
    resolved = resolveInspectionSource(options.input, cwd);
  } catch (error) {
    return emitFatal(error, false, options.json);
  }

  const client = new CompilerClient({
    cwd: resolved.root,
    ...(options.compilerPath === undefined ? {} : { binaryPath: options.compilerPath }),
  });
  const request = {
    source: { path: resolved.file.displayPath, text: resolved.source },
    target: options.target ?? "js",
    ...(options.mode === undefined ? {} : { mode: options.mode }),
  };

  let response;
  try {
    response =
      command === "ast" ? client.ast(request) : client.explain(request);
  } catch (error) {
    return emitFatal(error, false, options.json);
  }

  const diagnostics = response.diagnostics.map((diagnostic) =>
    reportDiagnostic(resolved.file, resolved.source, diagnostic),
  );
  const hasError = diagnostics.some((diagnostic) => diagnostic.severity === "error");

  if (options.json) {
    process.stdout.write(
      `${JSON.stringify({ ...response, diagnostics }, null, 2)}\n`,
    );
  } else {
    for (const diagnostic of diagnostics) {
      printDiagnostic(diagnostic);
    }
    if (!hasError) {
      if (command === "ast") {
        const ast = (response as AstResponse).ast;
        process.stdout.write(`${JSON.stringify(ast ?? null, null, 2)}\n`);
      } else {
        const explained = response as ExplainResponse;
        process.stdout.write(
          `${explained.explanation ?? JSON.stringify(explained.inference ?? null, null, 2)}\n`,
        );
      }
    }
  }

  return response.ok && !hasError ? 0 : 1;
}

function errorMessage(error: unknown): string {
  return error instanceof Error ? error.message : String(error);
}

function displayPath(path: string, cwd: string): string {
  const relativePath = relative(cwd, path);

  if (
    relativePath.length === 0 ||
    relativePath === ".." ||
    relativePath.startsWith(`..${sep}`)
  ) {
    return path.split(sep).join("/");
  }

  return relativePath.split(sep).join("/");
}

async function resolveNativeCompiler(
  buildData: Record<string, unknown>,
  projectRoot: string,
  allowDownload: boolean,
): Promise<{ bin: string; args: readonly string[]; family: string } | null> {
  let policy: ToolchainPolicy;
  try {
    policy = parseToolchainPolicy(buildData);
  } catch (error) {
    if (error instanceof ToolchainError) {
      throw new CliError(error.code as CliErrorCode, error.message);
    }
    throw error;
  }
  const probe: ToolchainProbe = {
    exists: (p: string) => existsSync(p),
    run: (cmd: string, a: readonly string[]) => {
      const r = spawnSync(cmd, [...a], { encoding: "utf8", windowsHide: true, input: a.includes("-") ? "#include <stdio.h>\nint main(void) { return 0; }\n" : undefined });
      return { status: r.status, stdout: r.stdout ?? "", stderr: r.stderr ?? r.error?.message ?? "" };
    },
    readFileSync: (p: string) => readFileSync(p, "utf8"),
    readdir: (p: string) => readdirSync(p),
  };
  const env = process.env["ASTRA_CC"] ? { ASTRA_CC: process.env["ASTRA_CC"] } : {};
  try {
    const resolution = await resolveToolchainAsync({
      projectRoot,
      policy,
      env,
      probe,
      allowDownload,
    });
    return { ...compilerInvocation(resolution), family: resolution.family };
  } catch (error) {
    if (error instanceof ToolchainError) {

      if (policy.request === "auto" && error.code === "NS9102") {
        return null;
      }
      throw new CliError(error.code as CliErrorCode, error.message);
    }
    if (policy.request === "auto") {
      return null;
    }
    throw new CliError("NS9008", errorMessage(error));
  }
}

function buildCflags(targetConfig: Record<string, unknown>): string[] {
  const cflags: string[] = [];
  if (Array.isArray(targetConfig["cflags"])) {
    for (const flag of targetConfig["cflags"]) {
      if (typeof flag === "string") {
        cflags.push(flag);
      }
    }
  }
  const optLevel = targetConfig["optLevel"] ?? targetConfig["optimization"];
  if (typeof optLevel === "string" || typeof optLevel === "number") {
    cflags.push(`-O${String(optLevel).replace(/^-?O?/, "")}`);
  }
  const triple = targetConfig["triple"] ?? targetConfig["targetTriple"];
  if (typeof triple === "string") {
    cflags.push(`--target=${triple}`);
  }
  return cflags;
}

function archiverCandidates(compiler: {
  bin: string;
  args: readonly string[];
  family: string;
}): string[] {
  const candidates: string[] = [];
  if (compiler.family === "zig") {

    candidates.push(compiler.bin);
  } else {
    const printed = spawnSync(compiler.bin, [...compiler.args, "-print-prog-name=ar"], {
      windowsHide: true,
      encoding: "utf8",
    });
    if (printed.status === 0) {
      const value = printed.stdout.trim();
      if (value.length > 0) {
        candidates.push(value);
      }
    }
  }
  for (const candidate of ["llvm-ar", "gcc-ar", "ar"]) {
    if (!candidates.includes(candidate)) {
      candidates.push(candidate);
    }
  }
  return candidates;
}

function invocationArgs(
  compilerArgs: readonly string[],
  family: string,
  cpp: boolean,
): string[] {
  if (family === "zig" && cpp) {
    return compilerArgs.map((argument) => (argument === "cc" ? "c++" : argument));
  }
  return [...compilerArgs];
}

function compilerFlavor(
  compilerBin: string,
  compilerArgs: readonly string[] = [],
): "msvc" | "gnu" {
  const probe = spawnSync(compilerBin, [...compilerArgs, "-dumpmachine"], {
    windowsHide: true,
    encoding: "utf8",
  });
  if (probe.status === 0 && probe.stdout.includes("windows-msvc")) {
    return "msvc";
  }
  return "gnu";
}

const EXPORTS_MARKER = "#define NS_ASTRA_EXPORTED_PROTOTYPES 1";

function exportedSymbolsFromHeader(headerPath: string): string[] {
  let text: string;
  try {
    text = readFileSync(headerPath, "utf8");
  } catch {
    return [];
  }
  const marker = text.indexOf(EXPORTS_MARKER);
  if (marker === -1) {
    return [];
  }
  const names: string[] = [];
  for (const line of text.slice(marker + EXPORTS_MARKER.length).split(/\r?\n/)) {
    const name = /\b([A-Za-z_][A-Za-z0-9_]*)\s*\([^;]*\)\s*;$/.exec(line)?.[1];
    if (name !== undefined) {
      names.push(name);
    }
  }
  return names;
}

function parseFmtArgs(args: readonly string[], cwd: string): FmtOptions {
  let check = false;
  let json = false;
  let compilerPath: string | undefined;
  const positional: string[] = [];

  for (let index = 0; index < args.length; index += 1) {
    const arg = args[index];

    if (arg === undefined) {
      continue;
    }

    if (arg === "--") {
      positional.push(...args.slice(index + 1));
      break;
    }

    if (arg === "--check") {
      check = true;
      continue;
    }

    if (arg === "--json") {
      json = true;
      continue;
    }

    if (arg === "--compiler") {
      const value = args[index + 1];
      if (value === undefined || value.startsWith("-")) {
        throw new CliError("NS9000", "The --compiler option requires a binary path.");
      }

      compilerPath = value;
      index += 1;
      continue;
    }

    if (arg.startsWith("-")) {
      throw new CliError("NS9000", `Unknown fmt option '${arg}'.`);
    }

    positional.push(arg);
  }

  if (positional.length > 1) {
    throw new CliError("NS9000", "astra fmt accepts at most one file or directory.");
  }

  return {
    path: resolve(cwd, positional[0] ?? "."),
    check,
    json,
    compilerPath,
  };
}

function parseBuildArgs(args: readonly string[], cwd: string): BuildOptions {
  let json = false;
  let target: string | undefined;
  let emit: string | undefined;
  let compilerPath: string | undefined;
  let noDownload = false;
  const positional: string[] = [];

  for (let index = 0; index < args.length; index += 1) {
    const arg = args[index];
    if (arg === undefined) {
      continue;
    }

    if (arg === "--") {
      positional.push(...args.slice(index + 1));
      break;
    }

    if (arg === "--json") {
      json = true;
      continue;
    }

    if (arg === "--no-download") {
      noDownload = true;
      continue;
    }

    if (arg === "--target" || arg === "--compiler" || arg === "--emit") {
      const value = args[index + 1];
      if (value === undefined || value.startsWith("-")) {
        throw new CliError("NS9000", `The ${arg} option requires a value.`);
      }
      if (arg === "--target") {
        target = value;
      } else if (arg === "--emit") {
        emit = value;
      } else {
        compilerPath = value;
      }
      index += 1;
      continue;
    }

    if (arg.startsWith("-")) {
      throw new CliError("NS9000", `Unknown build option '${arg}'.`);
    }

    positional.push(arg);
  }

  if (positional.length > 1) {
    throw new CliError("NS9000", "astra build accepts at most one project directory.");
  }

  return {
    project: resolve(cwd, positional[0] ?? "."),
    target,
    emit,
    json,
    compilerPath,
    noDownload,
  };
}

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}

function isMode(value: unknown): value is Mode {
  return value === "core" || value === "app" || value === "web" || value === "system";
}

function isIntegerPolicy(
  value: unknown,
): value is "wrapping" | "safe-checked" | "arbitrary" {
  return value === "wrapping" || value === "safe-checked" || value === "arbitrary";
}

function collectSourceFiles(inputPath: string, cwd: string): readonly SourceFile[] {
  let inputStat;

  try {
    inputStat = statSync(inputPath);
  } catch (error) {
    throw new CliError(
      "NS9008",
      `Unable to access the path: ${errorMessage(error)}`,
      displayPath(inputPath, cwd),
    );
  }

  if (inputStat.isFile()) {
    if (manifestFileForPath(inputPath) !== undefined) {
      return [
        {
          path: inputPath,
          displayPath: displayPath(inputPath, cwd),
          kind: "manifest",
        },
      ];
    }

    if (extname(inputPath).toLowerCase() !== SOURCE_EXTENSION) {
      throw new CliError(
        "NS9008",
        "The target is not a .ns source file or a core.json/core.toml manifest.",
        displayPath(inputPath, cwd),
      );
    }

    return [
      {
        path: inputPath,
        displayPath: displayPath(inputPath, cwd),
        kind: "source",
      },
    ];
  }

  if (!inputStat.isDirectory()) {
    throw new CliError(
      "NS9008",
      "The target is not a file or directory.",
      displayPath(inputPath, cwd),
    );
  }

  const files: SourceFile[] = [];
  const manifests = ["core.json", "core.toml"].filter((name) =>
    existsSync(join(inputPath, name)),
  );

  if (manifests.length > 1) {
    throw new CliError(
      "NS9001",
      "Both core.json and core.toml exist. Exactly one manifest is allowed.",
      displayPath(inputPath, cwd),
    );
  }

  const manifestName = manifests[0];
  if (manifestName !== undefined) {
    const manifestPath = join(inputPath, manifestName);
    files.push({
      path: manifestPath,
      displayPath: displayPath(manifestPath, cwd),
      kind: "manifest",
    });
  }

  const walk = (directory: string): void => {
    let entries;

    try {
      entries = readdirSync(directory, { withFileTypes: true });
    } catch (error) {
      throw new CliError(
        "NS9008",
        `Unable to read the directory: ${errorMessage(error)}`,
        displayPath(directory, cwd),
      );
    }

    entries.sort((left, right) => left.name.localeCompare(right.name));

    for (const entry of entries) {
      if (entry.isDirectory()) {
        const name = entry.name.toLowerCase();
        if (!entry.name.startsWith(".") && !IGNORED_DIRECTORIES.has(name)) {
          walk(join(directory, entry.name));
        }
        continue;
      }

      if (entry.isFile() && extname(entry.name).toLowerCase() === SOURCE_EXTENSION) {
        files.push({
          path: join(directory, entry.name),
          displayPath: displayPath(join(directory, entry.name), cwd),
          kind: "source",
        });
      }
    }
  };

  walk(inputPath);

  return files.sort((left, right) =>
    left.displayPath.localeCompare(right.displayPath),
  );
}

function lineColumn(
  source: string,
  byteOffset: number,
): { line: number; column: number } {
  const bytes = Buffer.from(source, "utf8");
  const end = Math.max(0, Math.min(byteOffset, bytes.length));
  const prefix = bytes.subarray(0, end).toString("utf8");
  const lines = prefix.split("\n");
  const last = lines[lines.length - 1] ?? "";

  return { line: lines.length, column: [...last].length + 1 };
}

function reportDiagnostic(
  file: SourceFile,
  source: string | null,
  diagnostic: CompilerDiagnostic,
): ReportedDiagnostic {
  const position =
    source !== null && diagnostic.span !== null
      ? lineColumn(source, diagnostic.span.start)
      : { line: null, column: null };

  return {
    code: diagnostic.code,
    severity: diagnostic.severity,
    message: diagnostic.message,
    hint: diagnostic.hint,
    path: file.displayPath,
    line: position.line,
    column: position.column,
  };
}

function errorDiagnostic(error: unknown, path: string | null = null): ReportedDiagnostic {
  if (error instanceof CliError) {
    return {
      code: error.code,
      severity: "error",
      message: error.message,
      hint: null,
      path: error.path ?? path,
      line: null,
      column: null,
    };
  }

  if (error instanceof ManifestError) {
    return {
      code: error.code,
      severity: "error",
      message: error.message,
      hint: null,
      path: error.path ?? path,
      line: null,
      column: null,
    };
  }

  if (error instanceof ToolchainError) {
    return {
      code: error.code,
      severity: "error",
      message: error.message,
      hint: null,
      path,
      line: null,
      column: null,
    };
  }

  if (error instanceof CompilerClientError) {
    return {
      code: error.code,
      severity: "error",
      message: error.message,
      hint: null,
      path,
      line: null,
      column: null,
    };
  }

  return {
    code: "NS9000",
    severity: "error",
    message: errorMessage(error),
    hint: null,
    path,
    line: null,
    column: null,
  };
}

function printDiagnostic(diagnostic: ReportedDiagnostic): void {
  const location =
    diagnostic.path === null
      ? null
      : diagnostic.line === null
        ? diagnostic.path
        : `${diagnostic.path}:${diagnostic.line}:${diagnostic.column ?? 1}`;
  const prefix = location === null ? "" : `${location}: `;
  const help = diagnostic.hint === null ? "" : `\n  help: ${diagnostic.hint}`;

  process.stderr.write(
    `${prefix}${diagnostic.severity}[${diagnostic.code}]: ${diagnostic.message}${help}\n`,
  );
}

function reportJson(
  check: boolean,
  results: readonly FileResult[],
  diagnostics: readonly ReportedDiagnostic[],
): void {
  const hasErrors = results.some((result) => result.status === "error");
  const hasChanges = results.some((result) => result.status === "would-reformat");

  process.stdout.write(
    `${JSON.stringify(
      {
        check,
        ok: !hasErrors && (!check || !hasChanges),
        files: results.map((result) => ({
          path: result.file.displayPath,
          status: result.status,
          diagnostics: result.diagnostics,
        })),
        diagnostics,
      },
      null,
      2,
    )}\n`,
  );
}

function emitFatal(error: unknown, check: boolean, json: boolean): number {
  const diagnostic = errorDiagnostic(error);
  if (json) {
    reportJson(check, [], [diagnostic]);
  } else {
    printDiagnostic(diagnostic);
  }

  return 1;
}

function readSource(file: SourceFile): string {
  let bytes: Buffer;

  try {
    bytes = readFileSync(file.path);
  } catch (error) {
    throw new CliError(
      "NS9008",
      `Unable to read the file: ${errorMessage(error)}`,
      file.displayPath,
    );
  }

  try {
    return new TextDecoder("utf-8", { fatal: true, ignoreBOM: true }).decode(bytes);
  } catch {
    throw new CliError(
      "NS9007",
      "Source is not valid UTF-8.",
      file.displayPath,
    );
  }
}

async function runFmt(args: readonly string[], cwd: string): Promise<number> {
  if (args.includes("-h") || args.includes("--help")) {
    printHelp();
    return 0;
  }

  let options: FmtOptions;
  try {
    options = parseFmtArgs(args, cwd);
  } catch (error) {
    return emitFatal(error, false, isJsonFlag(args));
  }

  let files: readonly SourceFile[];
  try {
    files = collectSourceFiles(options.path, cwd);
  } catch (error) {
    return emitFatal(error, options.check, options.json);
  }

  if (files.length === 0) {
    if (options.json) {
      reportJson(options.check, [], []);
    } else {
      process.stdout.write("No formattable files found.\n");
    }
    return 0;
  }

  let client: CompilerClient | undefined;
  const getClient = (): CompilerClient => {
    client ??= new CompilerClient({
      cwd,
      ...(options.compilerPath === undefined
        ? {}
        : { binaryPath: options.compilerPath }),
    });
    return client;
  };

  const results: FileResult[] = [];
  const allDiagnostics: ReportedDiagnostic[] = [];

  for (const file of files) {
    let source: string;
    try {
      source = readSource(file);
    } catch (error) {
      const diagnostic = errorDiagnostic(error, file.displayPath);
      allDiagnostics.push(diagnostic);
      results.push({
        file,
        status: "error",
        formatted: null,
        diagnostics: [diagnostic],
      });
      continue;
    }

    if (file.kind === "manifest") {
      const manifestFile = manifestFileForPath(file.path);
      if (manifestFile === undefined) {
        const diagnostic: ReportedDiagnostic = {
          code: "NS9002",
          severity: "error",
          message: "Unsupported manifest file name.",
          hint: null,
          path: file.displayPath,
          line: null,
          column: null,
        };
        allDiagnostics.push(diagnostic);
        results.push({
          file,
          status: "error",
          formatted: null,
          diagnostics: [diagnostic],
        });
        continue;
      }

      let formatted: string;
      try {
        formatted = await formatManifestSource(source, manifestFile);
      } catch (error) {
        const diagnostic = errorDiagnostic(error, file.displayPath);
        allDiagnostics.push(diagnostic);
        results.push({
          file,
          status: "error",
          formatted: null,
          diagnostics: [diagnostic],
        });
        continue;
      }

      const changed = formatted !== source;
      results.push({
        file,
        status: changed
          ? options.check
            ? "would-reformat"
            : "formatted"
          : "unchanged",
        formatted: changed ? formatted : null,
        diagnostics: [],
      });
      continue;
    }

    let response;
    try {
      response = getClient().formatSource(file.displayPath, source);
    } catch (error) {
      const diagnostic = errorDiagnostic(error, file.displayPath);
      allDiagnostics.push(diagnostic);
      results.push({
        file,
        status: "error",
        formatted: null,
        diagnostics: [diagnostic],
      });
      break;
    }

    const fileDiagnostics = response.diagnostics.map((diagnostic) =>
      reportDiagnostic(file, source, diagnostic),
    );
    allDiagnostics.push(...fileDiagnostics);

    if (
      response.source === null &&
      !fileDiagnostics.some((diagnostic) => diagnostic.severity === "error")
    ) {
      const diagnostic: ReportedDiagnostic = {
        code: "NS9005",
        severity: "error",
        message: "Compiler returned no formatted source.",
        hint: null,
        path: file.displayPath,
        line: null,
        column: null,
      };
      fileDiagnostics.push(diagnostic);
      allDiagnostics.push(diagnostic);
    }

    if (
      response.source === null ||
      fileDiagnostics.some((diagnostic) => diagnostic.severity === "error")
    ) {
      results.push({
        file,
        status: "error",
        formatted: null,
        diagnostics: fileDiagnostics,
      });
      continue;
    }

    const changed = response.source !== source;
    const status: FileStatus = changed
      ? options.check
        ? "would-reformat"
        : "formatted"
      : "unchanged";
    results.push({
      file,
      status,
      formatted: changed ? response.source : null,
      diagnostics: fileDiagnostics,
    });
  }

  if (results.some((result) => result.status === "error")) {
    if (options.json) {
      reportJson(options.check, results, allDiagnostics);
    } else {
      for (const diagnostic of allDiagnostics) {
        printDiagnostic(diagnostic);
      }
    }
    return 1;
  }

  if (!options.json) {
    for (const diagnostic of allDiagnostics) {
      printDiagnostic(diagnostic);
    }
  }

  if (options.check) {
    if (options.json) {
      reportJson(options.check, results, allDiagnostics);
    } else {
      const changed = results.filter((result) => result.status === "would-reformat");
      for (const result of changed) {
        process.stdout.write(`would reformat ${result.file.displayPath}\n`);
      }

      process.stdout.write(
        changed.length === 0
          ? `All ${results.length} file(s) are formatted.\n`
          : `${changed.length} file(s) need formatting.\n`,
      );
    }

    return results.some((result) => result.status === "would-reformat") ? 1 : 0;
  }

  const changed = results.filter((result) => result.status === "formatted");
  for (const result of changed) {
    if (result.formatted === null) {
      continue;
    }

    try {
      writeFileSync(result.file.path, result.formatted, "utf8");
    } catch (error) {
      return emitFatal(
        new CliError(
          "NS9014",
          `Unable to write the file: ${errorMessage(error)}`,
          result.file.displayPath,
        ),
        options.check,
        options.json,
      );
    }
  }

  if (options.json) {
    reportJson(options.check, results, allDiagnostics);
  } else {
    for (const result of changed) {
      process.stdout.write(`formatted ${result.file.displayPath}\n`);
    }

    process.stdout.write(
      changed.length === 0
        ? `All ${results.length} file(s) already formatted.\n`
        : `${changed.length} file(s) formatted.\n`,
    );
  }

  return 0;
}

function printBuildReport(report: BuildReport): void {
  process.stdout.write(`${JSON.stringify(report, null, 2)}\n`);
}

function codegenDiagnostic(
  file: SourceFile,
  source: string,
  diagnostic: { readonly code: string; readonly message: string; readonly span?: { readonly start: number } },
): ReportedDiagnostic {
  const position =
    diagnostic.span === undefined
      ? { line: null, column: null }
      : lineColumn(source, diagnostic.span.start);

  return {
    code: diagnostic.code,
    severity: "error",
    message: diagnostic.message,
    hint: null,
    path: file.displayPath,
    line: position.line,
    column: position.column,
  };
}

interface ProjectModuleSource {
  readonly key: string;
  readonly absolutePath: string;
  readonly text: string;
}

function scanImportSpecifiers(source: string): readonly string[] {
  const specifiers = new Set<string>();
  const fromPattern = /^[ \t]*(?:import|export)\b[^\n;]*?\bfrom\s*"([^"]+)"/gm;
  const sideEffectPattern = /^[ \t]*import\s*"([^"]+)"/gm;
  const pathPattern =
    /^[ \t]*import\s+([A-Za-z_][A-Za-z0-9_]*(?:\.[A-Za-z_][A-Za-z0-9_]*)+)[ \t]*(?:;)?$/gm;
  for (const pattern of [fromPattern, sideEffectPattern]) {
    for (const match of source.matchAll(pattern)) {
      if (match[1] !== undefined) {
        specifiers.add(match[1]);
      }
    }
  }
  for (const match of source.matchAll(pathPattern)) {
    if (match[1] !== undefined) {
      specifiers.add(match[1]);
    }
  }
  return [...specifiers];
}

function resolveProjectSpecifier(
  importerKey: string,
  specifier: string,
  root: string,
): string | undefined {
  if (specifier.startsWith("./") || specifier.startsWith("../")) {
    let target = resolve(dirname(join(root, importerKey)), specifier);
    if (!target.toLowerCase().endsWith(SOURCE_EXTENSION)) {
      target = `${target}${SOURCE_EXTENSION}`;
    }
    const relativeTarget = relative(root, target);
    if (
      relativeTarget.length === 0 ||
      isAbsolute(relativeTarget) ||
      relativeTarget === ".." ||
      relativeTarget.startsWith(`..${sep}`)
    ) {
      return undefined;
    }
    return relativeTarget.split(sep).join("/");
  }

  if (
    !specifier.includes("/") &&
    specifier.includes(".") &&
    !specifier.toLowerCase().endsWith(SOURCE_EXTENSION)
  ) {
    return `${specifier.replaceAll(".", "/")}${SOURCE_EXTENSION}`;
  }

  return undefined;
}

function discoverProjectModules(
  root: string,
  entryPath: string,
  entryKey: string,
  cwd: string,
): readonly ProjectModuleSource[] {
  const ordered: ProjectModuleSource[] = [];
  const visited = new Set<string>();

  const visit = (key: string, absolutePath: string): void => {
    if (visited.has(key)) {
      return;
    }
    visited.add(key);
    const file: SourceFile = {
      path: absolutePath,
      displayPath: displayPath(absolutePath, cwd),
      kind: "source",
    };
    const text = readSource(file);
    for (const specifier of scanImportSpecifiers(text)) {
      const targetKey = resolveProjectSpecifier(key, specifier, root);
      if (targetKey === undefined || visited.has(targetKey)) {
        continue;
      }
      const targetPath = resolve(root, targetKey);
      if (!existsSync(targetPath)) {
        continue;
      }
      visit(targetKey, targetPath);
    }
    ordered.push({ key, absolutePath, text });
  };

  visit(entryKey, entryPath);
  return ordered;
}

interface LinkedPathPackages {
  readonly sources: readonly ProjectModuleSource[];
  readonly importMap: Readonly<Record<string, string>>;
}

function readJsonIfExists(path: string): unknown {
  if (!existsSync(path)) {
    return undefined;
  }
  try {
    return JSON.parse(readFileSync(path, "utf8"));
  } catch {
    return undefined;
  }
}

function loadInstalledPackageRoots(root: string): Map<string, string> {
  const roots = new Map<string, string>();
  const lockfile = readJsonIfExists(join(root, "core.lock.json"));
  const store = readJsonIfExists(join(root, ".astra", "store", "index.json"));
  if (!isRecord(lockfile) || !isRecord(store)) {
    return roots;
  }
  if (!Array.isArray(lockfile["packages"]) || !Array.isArray(store["packages"])) {
    return roots;
  }

  const storeRootById = new Map<string, string>();
  for (const entry of store["packages"]) {
    if (
      isRecord(entry) &&
      typeof entry["id"] === "string" &&
      typeof entry["root"] === "string"
    ) {
      storeRootById.set(entry["id"], resolve(root, entry["root"]));
    }
  }

  for (const entry of lockfile["packages"]) {
    if (
      isRecord(entry) &&
      typeof entry["name"] === "string" &&
      typeof entry["id"] === "string"
    ) {
      const installed = storeRootById.get(entry["id"]);
      if (installed !== undefined) {
        roots.set(entry["name"], installed);
      }
    }
  }
  return roots;
}

function linkedPathPackages(
  root: string,
  manifest: ManifestDocument,
  targetName: string,
  backend: string,
  cwd: string,
): LinkedPathPackages {
  const dependencies = isRecord(manifest.data["dependencies"])
    ? manifest.data["dependencies"]
    : {};
  const installedRoots = loadInstalledPackageRoots(root);
  const sources: ProjectModuleSource[] = [];
  const importMap: Record<string, string> = {};
  const seen = new Set<string>();

  for (const [name, raw] of Object.entries(dependencies)) {
    if (!isRecord(raw)) {
      continue;
    }
    const isPath = raw["source"] === "path" && typeof raw["path"] === "string";
    const targets = Array.isArray(raw["targets"])
      ? raw["targets"].filter((value): value is string => typeof value === "string")
      : undefined;
    if (
      targets !== undefined &&
      !targets.includes(targetName) &&
      !targets.includes(backend)
    ) {
      continue;
    }

    let packageRoot: string;
    if (isPath) {
      packageRoot = resolve(root, raw["path"] as string);
      if (!existsSync(packageRoot)) {
        throw new CliError(
          "NS9016",
          `Path dependency '${name}' does not exist: ${displayPath(packageRoot, cwd)}`,
          displayPath(packageRoot, cwd),
        );
      }
    } else {
      const installed = installedRoots.get(name);
      if (installed === undefined || !existsSync(installed)) {
        continue;
      }
      packageRoot = installed;
    }
    let packageManifest: ManifestDocument;
    let packageBuildManifest: ManifestDocument | undefined;
    try {
      packageManifest = loadManifest(packageRoot);
      packageBuildManifest = loadBuildManifest(packageRoot);
    } catch (error) {

      if (isPath) {
        throw error;
      }
      continue;
    }
    const packageDir = dirname(packageManifest.file.path);
    const packageData = { ...packageManifest.data, ...(packageBuildManifest?.data ?? {}) };
    const nova = isRecord(packageData["nova"])
      ? packageData["nova"]
      : {};
    const entryValue = nova["entry"];
    if (typeof entryValue !== "string" || entryValue.length === 0) {
      if (isPath) {
        throw new CliError(
          "NV0002",
          `Dependency '${name}' does not declare nova.entry.`,
          displayPath(packageManifest.file.path, cwd),
        );
      }
      continue;
    }

    const entryPath = resolve(packageDir, entryValue);
    const relativeEntry = relative(packageDir, entryPath);
    if (
      relativeEntry.length === 0 ||
      isAbsolute(relativeEntry) ||
      relativeEntry === ".." ||
      relativeEntry.startsWith(`..${sep}`)
    ) {
      if (isPath) {
        throw new CliError(
          "NS9016",
          `Path dependency '${name}' has an entry outside its package.`,
          displayPath(packageManifest.file.path, cwd),
        );
      }
      continue;
    }
    const entryKey = relativeEntry.split(sep).join("/");
    const namespace = `deps/${name.replace(/[^A-Za-z0-9_.-]/g, "_")}`;
    importMap[name] = `${namespace}/${entryKey}`;

    for (const module of discoverProjectModules(
      packageDir,
      entryPath,
      entryKey,
      cwd,
    )) {
      const key = `${namespace}/${module.key}`;
      if (seen.has(key)) {
        continue;
      }
      seen.add(key);
      sources.push({ key, absolutePath: module.absolutePath, text: module.text });
    }
  }

  return { sources, importMap };
}

function outputPathForModule(  outDir: string,
  key: string,
  extension: "js" | "ts" = "js",
): string {
  return resolve(outDir, key.replace(/\.ns$/i, `.${extension}`));
}

function relativeModuleSpecifier(fromOutput: string, targetOutput: string): string {
  const value = relative(dirname(fromOutput), targetOutput).split(sep).join("/");
  return value.startsWith(".") ? value : `./${value}`;
}

async function runBuild(args: readonly string[], cwd: string): Promise<number> {
  if (args.includes("-h") || args.includes("--help")) {
    printHelp();
    return 0;
  }

  let options: BuildOptions;
  try {
    options = parseBuildArgs(args, cwd);
  } catch (error) {
    const diagnostic = errorDiagnostic(error);
    if (isJsonFlag(args)) {
      printBuildReport({
        target: "(default)",
        backend: "js",
        entry: "",
        output: null,
        diagnostics: [diagnostic],
      });
    } else {
      printDiagnostic(diagnostic);
    }
    return 1;
  }

  let report: BuildReport = {
    target: options.target ?? "(default)",
    backend: "js",
    entry: "",
    output: null,
    diagnostics: [],
  };

  try {
    const manifests = loadProjectManifests(options.project);
    const manifest = manifests.core;
    const root = dirname(manifest.file.path);
    const buildDoc = manifests.build;
    const buildData = applyBuildDefaults(buildDoc?.data);
    const build = isRecord(buildData["build"]) ? buildData["build"] : {};
    const targets = isRecord(buildData["targets"])
      ? buildData["targets"]
      : {};
    const targetName =
      options.target ??
      process.env.ASTRA_TARGET ??
      (typeof build["defaultTarget"] === "string" ? build["defaultTarget"] : undefined);

    if (targetName === undefined || targetName.length === 0) {
      throw new CliError(
        "NS9011",
        "No build target was selected. Set --target, ASTRA_TARGET, or build.defaultTarget.",
        displayPath(manifest.file.path, cwd),
      );
    }

    const targetConfig = isRecord(targets[targetName]) ? targets[targetName] : {};
    if (
      !isRecord(targets[targetName]) &&
      targetName !== "c" &&
      targetName !== "native" &&
      targetName !== "ts"
    ) {
      throw new CliError(
        "NS9011",
        `Target '${targetName}' is not declared in the manifest.`,
        displayPath(manifest.file.path, cwd),
      );
    }

    const backend =
      targetConfig["backend"] ??
      (targetName === "c" || targetName === "native" || targetName === "ts"
        ? targetName
        : "js");
    if (
      backend !== "js" &&
      backend !== "ts" &&
      backend !== "c" &&
      backend !== "native"
    ) {
      throw new CliError(
        "NS8001",
        `Target '${targetName}' uses backend '${String(backend)}'; astra build currently supports 'js', 'ts', 'c', and 'native' backends.`,
        displayPath(manifest.file.path, cwd),
      );
    }

    if (backend === "js" || backend === "ts") {
      const moduleFormat = targetConfig["module"] ?? "esm";
      if (moduleFormat !== "esm") {
        throw new CliError(
          "NS8001",
          `Target '${targetName}' requests module format '${String(moduleFormat)}'; only 'esm' is supported.`,
          displayPath(manifest.file.path, cwd),
        );
      }
    }

    const emitValue = options.emit ?? targetConfig["emit"] ?? "executable";
    const emitKind =
      emitValue === "library" || emitValue === "lib" ? "staticlib" : emitValue;
    if (
      emitKind !== "executable" &&
      emitKind !== "staticlib" &&
      emitKind !== "cdylib"
    ) {
      throw new CliError(
        "NS9012",
        `Unknown emit kind '${String(emitValue)}'. Use executable, staticlib, cdylib, or library.`,
        displayPath(manifest.file.path, cwd),
      );
    }
    const libraryMode = emitKind !== "executable";
    if (libraryMode && backend !== "c" && backend !== "native") {
      throw new CliError(
        "NS8001",
        `Target '${targetName}' uses backend '${String(backend)}'; library emission requires the 'c' or 'native' backend.`,
        displayPath(manifest.file.path, cwd),
      );
    }

    const nova = isRecord(buildData["nova"]) ? buildData["nova"] : {};
    const entryValue = nova["entry"];
    if (typeof entryValue !== "string" || entryValue.length === 0) {
      throw new CliError(
        "NV0002",
        "The manifest does not declare nova.entry.",
        displayPath(manifest.file.path, cwd),
      );
    }

    const entryPath = resolve(root, entryValue);
    const relativeEntry = relative(root, entryPath);
    if (
      relativeEntry.length === 0 ||
      isAbsolute(relativeEntry) ||
      relativeEntry === ".." ||
      relativeEntry.startsWith(`..${sep}`)
    ) {
      throw new CliError(
        "NS9013",
        "The configured entry point must be inside the project.",
        displayPath(entryPath, cwd),
      );
    }
    if (extname(entryPath).toLowerCase() !== SOURCE_EXTENSION) {
      throw new CliError(
        "NS9013",
        "The configured entry point must be a .ns file.",
        displayPath(entryPath, cwd),
      );
    }

    const entryKey = relativeEntry.split(sep).join("/");
    const sourceFile: SourceFile = {
      path: entryPath,
      displayPath: displayPath(entryPath, cwd),
      kind: "source",
    };
    const projectSources = discoverProjectModules(root, entryPath, entryKey, cwd);
    const source =
      projectSources.find((module) => module.key === entryKey)?.text ??
      readSource(sourceFile);

    const modeValue = targetConfig["mode"] ?? build["defaultMode"];
    if (modeValue !== undefined && !isMode(modeValue)) {
      throw new CliError(
        "NS9012",
        `Unknown mode '${String(modeValue)}' for target '${targetName}'.`,
        displayPath(manifest.file.path, cwd),
      );
    }

    const math = isRecord(buildData["math"]) ? buildData["math"] : {};
    const policyValue = math["integers"];
    if (policyValue !== undefined && !isIntegerPolicy(policyValue)) {
      throw new CliError(
        "NS9012",
        `Unknown integer policy '${String(policyValue)}'.`,
        displayPath(manifest.file.path, cwd),
      );
    }

    const client = new CompilerClient({
      cwd: root,
      ...(options.compilerPath === undefined
        ? {}
        : { binaryPath: options.compilerPath }),
    });
    const compilerTarget =
      backend === "c" || backend === "native" ? "c" : backend === "ts" ? "ts" : "js";
    const sharedParams = {
      ...(modeValue === undefined ? {} : { mode: modeValue }),
      ...(policyValue === undefined ? {} : { integerPolicy: policyValue }),
    };

    if (compilerTarget === "c") {

      type CBuildFile = {
        filename: string;
        content: string;
        kind: "source" | "header" | "host" | "script";
        isEntry?: boolean;
      };
      let files: CBuildFile[];
      let diagnostics: ReportedDiagnostic[];
      const linkedPackages = linkedPathPackages(
        root,
        manifest,
        targetName,
        String(backend),
        cwd,
      );
      const cSources = [...linkedPackages.sources, ...projectSources];

      if (cSources.length <= 1 && !libraryMode) {
        const single = client.compile({
          source: { path: entryKey, text: source },
          target: "c",
          ...sharedParams,
        });
        diagnostics = single.diagnostics.map((diagnostic) =>
          reportDiagnostic(sourceFile, source, diagnostic),
        );
        files = [];
        for (const artifact of single.artifacts) {
          if (!isRecord(artifact)) continue;
          const artType = artifact["type"];
          const artName = artifact["name"] ?? artifact["filename"];
          const artContent = artifact["content"];
          if (typeof artName !== "string" || typeof artContent !== "string") continue;
          const kind =
            artType === "c-host-shim"
              ? "host"
              : artType === "c-script"
                ? "script"
                : "source";
          files.push({ filename: artName, content: artContent, kind, isEntry: kind === "source" });
        }
      } else {
        const project = client.compileProject({
          sources: cSources.map((module) => ({
            key: module.key,
            path: module.key,
            text: module.text,
          })),
          target: "c",
          ...sharedParams,
          ...(libraryMode ? { library: true } : {}),
          ...(Object.keys(linkedPackages.importMap).length === 0
            ? {}
            : { importMap: linkedPackages.importMap }),
        });
        diagnostics = project.diagnostics.map((diagnostic) =>
          reportDiagnostic(sourceFile, source, diagnostic),
        );
        files = [];
        for (const module of project.modules) {
          files.push({
            filename: module.cFilename,
            content: module.cSource,
            kind: "source",
            isEntry: module.key === entryKey,
          });
          files.push({ filename: module.hFilename, content: module.hSource, kind: "header" });
          for (const [scriptName, scriptContent] of module.scriptSources) {
            files.push({ filename: scriptName, content: scriptContent, kind: "script" });
          }
        }
        if (project.hostShim.length > 0) {
          files.push({ filename: "astra_host.c", content: project.hostShim, kind: "host" });
        }
      }

      const errors = diagnostics.filter(
        (diagnostic) => diagnostic.severity === "error",
      );
      if (errors.length > 0) {
        report = {
          ...report,
          target: targetName,
          backend: String(backend),
          entry: entryKey,
          diagnostics,
        };
        if (options.json) {
          printBuildReport(report);
        } else {
          for (const diagnostic of diagnostics) {
            printDiagnostic(diagnostic);
          }
        }
        return 1;
      }
      const configuredOutDir = targetConfig["outDir"] ?? build["outDir"];
      const outDir =
        typeof configuredOutDir === "string"
          ? resolve(root, configuredOutDir)
          : resolve(root, "dist", targetName);

      let mainCPath = "";
      let hostCPath = "";
      const cSourcePaths: string[] = [];
      const scriptPaths: string[] = [];
      const headerPaths: string[] = [];
      try {
        mkdirSync(outDir, { recursive: true });

        for (const file of files) {
          const filePath = resolve(outDir, file.filename);
          mkdirSync(dirname(filePath), { recursive: true });
          writeFileSync(filePath, file.content, "utf8");
          if (file.kind === "source") {
            cSourcePaths.push(filePath);
            if (file.isEntry === true) mainCPath = filePath;
          } else if (file.kind === "host") {
            hostCPath = filePath;
          } else if (file.kind === "script") {
            scriptPaths.push(filePath);
          } else if (file.kind === "header") {
            headerPaths.push(filePath);
          }
        }
      } catch (error) {
        throw new CliError(
          "NS9014",
          `Unable to write generated C artifacts: ${errorMessage(error)}`,
          displayPath(outDir, cwd),
        );
      }

      let outputPath: string = mainCPath || outDir;

      if (libraryMode) {
        const compilerInv = await resolveNativeCompiler(buildData, root, !options.noDownload);
        const compilerBin = compilerInv?.bin ?? null;
        if (!compilerBin) {
          throw new CliError(
            "NS9102",
            "No C compiler (clang, gcc, or cc) found on PATH to build a library.",
            displayPath(outDir, cwd),
          );
        }
        const compilerArgs = compilerInv?.args ?? [];
        const compilerFamily = compilerInv?.family ?? "unknown";
        if (!mainCPath || !hostCPath) {
          throw new CliError(
            "NS8002",
            "Compiler did not produce the expected C artifacts for library emission.",
            displayPath(outDir, cwd),
          );
        }
        const cflags = buildCflags(targetConfig);
        const objDir = resolve(outDir, "obj");
        try {
          mkdirSync(objDir, { recursive: true });
        } catch (error) {
          throw new CliError(
            "NS9014",
            `Unable to create the object directory: ${errorMessage(error)}`,
            displayPath(objDir, cwd),
          );
        }

        const compileObject = (source: string, object: string): void => {
          const isCpp = source.endsWith(".cpp");
          const sourceInvocation = invocationArgs(
            compilerArgs,
            compilerFamily,
            isCpp,
          );
          const sourceLabel =
            compilerFamily === "zig"
              ? `${compilerBin} ${isCpp ? "c++" : "cc"}`
              : compilerBin;

          const pic =
            emitKind === "cdylib" &&
            process.platform !== "win32" &&
            compilerFlavor(compilerBin, sourceInvocation) !== "msvc"
              ? ["-fPIC"]
              : [];
          const proc = spawnSync(
            compilerBin,
            [
              ...sourceInvocation,
              isCpp ? "-std=c++17" : "-std=c17",
              "-Wall",
              "-Wno-unused-variable",
              ...pic,
              ...cflags,
              "-c",
              "-o",
              object,
              source,
            ],
            { windowsHide: true, encoding: "utf8" },
          );
          if (proc.status !== 0) {
            throw new CliError(
              "NS9015",
              `Library object compilation failed with ${sourceLabel} for ${basename(source)}:\n${proc.stderr}`,
              displayPath(outDir, cwd),
            );
          }
        };

        const objects: string[] = [];
        for (const source of [...cSourcePaths, ...scriptPaths]) {
          const object = resolve(
            objDir,
            `${basename(source, extname(source))}.o`,
          );
          compileObject(source, object);
          objects.push(object);
        }

        const packageName = getPackageName(manifest) ?? targetName;
        const libStem = `lib${packageName.replace(/[^A-Za-z0-9_.-]/g, "_")}`;
        const hasCpp = scriptPaths.some((path) => path.endsWith(".cpp"));

        if (emitKind === "staticlib") {

          const archive = resolve(outDir, `${libStem}.a`);
          const failures: string[] = [];
          let archiver: string | null = null;
          for (const candidate of archiverCandidates({
            bin: compilerBin,
            args: compilerArgs,
            family: compilerFamily,
          })) {
            rmSync(archive, { force: true });
            const isZig = compilerFamily === "zig" && candidate === compilerBin;
            const arArgs = isZig
              ? ["ar", "rcs", archive, ...objects]
              : ["rcs", archive, ...objects];
            const proc = spawnSync(candidate, arArgs, {
              windowsHide: true,
              encoding: "utf8",
            });
            if (proc.status === 0) {
              archiver = candidate;
              break;
            }
            failures.push(
              `${candidate}: ${
                proc.stderr?.trim() ||
                proc.error?.message ||
                `exit ${String(proc.status)}`
              }`,
            );
          }
          if (!archiver) {
            rmSync(archive, { force: true });
            throw new CliError(
              "NS9102",
              "No archiver (zig ar, ar, llvm-ar, or gcc-ar) could build a static library:\n" +
                failures.join("\n"),
              displayPath(outDir, cwd),
            );
          }
          outputPath = archive;
        } else {

          const hostObject = resolve(objDir, "astra_host.o");
          compileObject(hostCPath, hostObject);
          const sharedExt = process.platform === "win32" ? ".dll" : ".so";
          const shared = resolve(outDir, `${libStem}${sharedExt}`);
          const linkArgs = ["-shared", "-o", shared, ...objects, hostObject];
          const linkInvocation = invocationArgs(
            compilerArgs,
            compilerFamily,
            hasCpp,
          );
          const linkLabel =
            compilerFamily === "zig"
              ? `${compilerBin} ${hasCpp ? "c++" : "cc"}`
              : compilerBin;
          const flavor = compilerFlavor(compilerBin, linkInvocation);
          if (flavor === "msvc") {
            const exports = [
              ...new Set(headerPaths.flatMap(exportedSymbolsFromHeader)),
            ];
            if (exports.length > 0) {
              const defPath = resolve(outDir, `${libStem}.def`);
              writeFileSync(
                defPath,
                `EXPORTS\n${exports.join("\n")}\n`,
                "utf8",
              );
              linkArgs.push(`-Wl,/DEF:${defPath}`);
            }
            linkArgs.push(
              `-Wl,/IMPLIB:${resolve(outDir, `${libStem}.lib`)}`,
            );
          } else if (process.platform === "win32") {
            linkArgs.push(
              "-Wl,--export-all-symbols",
              `-Wl,--out-implib,${resolve(outDir, `${libStem}.dll.a`)}`,
            );
          }
          if (hasCpp && flavor === "gnu") {
            linkArgs.push("-lstdc++");
          }
          const proc = spawnSync(compilerBin, [...linkInvocation, ...linkArgs], {
            windowsHide: true,
            encoding: "utf8",
          });
          if (proc.status !== 0) {
            throw new CliError(
              "NS9015",
              `Shared library link failed with ${linkLabel}:\n${proc.stderr}`,
              displayPath(outDir, cwd),
            );
          }
          outputPath = shared;
        }
      } else if (backend === "native") {
        if (!mainCPath || !hostCPath) {
          throw new CliError(
            "NS8002",
            "Compiler did not produce the expected C artifacts for native compilation.",
            displayPath(outDir, cwd),
          );
        }
        const exeName = process.platform === "win32" ? "main.exe" : "main";
        const exePath = resolve(outDir, exeName);
        const compilerInv = await resolveNativeCompiler(buildData, root, !options.noDownload);
        const compilerBin = compilerInv?.bin ?? null;
        if (!compilerBin) {
          throw new CliError(
            "NS9102",
            "No C compiler (clang, gcc, or cc) found on PATH to build native binary.",
            displayPath(outDir, cwd),
          );
        }
        const compilerArgs = compilerInv?.args ?? [];
        const compilerFamily = compilerInv?.family ?? "unknown";
        const cflags = buildCflags(targetConfig);
        const hasCpp = scriptPaths.some((p) => p.endsWith(".cpp"));
        const linkInvocation = invocationArgs(
          compilerArgs,
          compilerFamily,
          hasCpp,
        );
        const linkLabel =
          compilerFamily === "zig"
            ? `${compilerBin} ${hasCpp ? "c++" : "cc"}`
            : compilerBin;
        const compileArgs = [
          "-std=c17",
          "-Wall",
          "-Wno-unused-variable",
          ...cflags,
          "-o",
          exePath,
          ...cSourcePaths,
          hostCPath,
          ...scriptPaths,
        ];
        if (hasCpp) {
          compileArgs.push("-lstdc++");
        }
        const buildProc = spawnSync(
          compilerBin,
          [...linkInvocation, ...compileArgs],
          { windowsHide: true, encoding: "utf8" },
        );
        if (buildProc.status !== 0) {
          throw new CliError(
            "NS9015",
            `Native compilation failed with ${linkLabel}:\n${buildProc.stderr}`,
            displayPath(outDir, cwd),
          );
        }
        outputPath = exePath;
      }

      report = {
        target: targetName,
        backend: String(backend),
        entry: entryKey,
        output: displayPath(outputPath, cwd),
        diagnostics,
      };
      if (options.json) {
        printBuildReport(report);
      } else {
        for (const diagnostic of diagnostics) {
          printDiagnostic(diagnostic);
        }
        process.stdout.write(
          `built ${targetName} -> ${displayPath(outputPath, cwd)}\n`,
        );
      }
      return 0;
    }

    const result = client.checkProject({
      sources: projectSources.map((module) => ({
        key: module.key,
        path: module.key,
        text: module.text,
      })),
      target: compilerTarget,
      ...sharedParams,
    });

    const moduleSources = new Map(
      projectSources.map((module) => [module.key, module] as const),
    );
    const diagnostics = result.modules.flatMap((module) => {
      const moduleSource = moduleSources.get(module.key);
      const file: SourceFile = {
        path: moduleSource?.absolutePath ?? entryPath,
        displayPath:
          moduleSource === undefined
            ? displayPath(entryPath, cwd)
            : displayPath(moduleSource.absolutePath, cwd),
        kind: "source",
      };
      return module.diagnostics.map((diagnostic) =>
        reportDiagnostic(file, moduleSource?.text ?? source, diagnostic),
      );
    });
    const errors = diagnostics.filter((diagnostic) => diagnostic.severity === "error");
    if (errors.length > 0) {
      report = { ...report, target: targetName, entry: entryKey, diagnostics };
      if (options.json) {
        printBuildReport(report);
      } else {
        for (const diagnostic of diagnostics) {
          printDiagnostic(diagnostic);
        }
      }
      return 1;
    }

    const missingModule = result.modules.find(
      (module) => module.ok && module.ir === undefined,
    );
    if (missingModule !== undefined) {
      throw new CliError(
        "NS8002",
        `The compiler returned no typed module for '${missingModule.key}'.`,
        displayPath(entryPath, cwd),
      );
    }

    const configuredOutDir = targetConfig["outDir"] ?? build["outDir"];
    const outDir =
      typeof configuredOutDir === "string"
        ? resolve(root, configuredOutDir)
        : resolve(root, "dist", targetName);

    const outputExtension: "js" | "ts" = compilerTarget === "ts" ? "ts" : "js";
    const outputs = new Map(
      projectSources.map((module) => [
        module.key,
        outputPathForModule(outDir, module.key, outputExtension),
      ]),
    );
    const specifierPaths = new Map(
      projectSources.map((module) => [
        module.key,
        outputPathForModule(outDir, module.key, "js"),
      ]),
    );

    const generatedModules = result.modules
      .filter(
        (module): module is typeof module & { readonly ir: NonNullable<typeof module.ir> } =>
          module.ir !== undefined,
      )
      .map((module) => {
        const outputPath = outputs.get(module.key);
        const moduleSource = moduleSources.get(module.key);
        if (outputPath === undefined || moduleSource === undefined) {
          throw new CliError(
            "NS8002",
            `No output path was resolved for '${module.key}'.`,
            displayPath(entryPath, cwd),
          );
        }
        const moduleSpecifiers = Object.fromEntries(
          projectSources.map((candidate) => [
            candidate.key,
            relativeModuleSpecifier(
              specifierPaths.get(module.key) ?? outputPath,
              specifierPaths.get(candidate.key) ??
                outputPathForModule(outDir, candidate.key, "js"),
            ),
          ]),
        );
        const generated =
          compilerTarget === "ts"
            ? generateTypeScript({
                moduleName: module.key,
                ir: module.ir,
                sourceText: moduleSource.text,
                moduleSpecifiers,
              })
            : generateJavaScript({
                moduleName: module.key,
                ir: module.ir,
                sourceText: moduleSource.text,
                moduleSpecifiers,
              });
        return {
          module,
          moduleSource,
          outputPath,
          generated,
        };
      });

    const generatedDiagnostics = generatedModules.flatMap((entry) =>
      entry.generated.diagnostics.map((diagnostic) =>
        codegenDiagnostic(
          {
            path: entry.moduleSource.absolutePath,
            displayPath: displayPath(entry.moduleSource.absolutePath, cwd),
            kind: "source",
          },
          entry.moduleSource.text,
          diagnostic,
        ),
      ),
    );
    const allDiagnostics = [...diagnostics, ...generatedDiagnostics];
    if (generatedDiagnostics.length > 0) {
      report = {
        ...report,
        target: targetName,
        entry: entryKey,
        diagnostics: allDiagnostics,
      };
      if (options.json) {
        printBuildReport(report);
      } else {
        for (const diagnostic of allDiagnostics) {
          printDiagnostic(diagnostic);
        }
      }
      return 1;
    }

    const outputPath = outputs.get(entryKey);
    if (outputPath === undefined) {
      throw new CliError(
        "NS8002",
        "The entry module was not included in the project output.",
        displayPath(entryPath, cwd),
      );
    }

    try {
      for (const entry of generatedModules) {
        mkdirSync(dirname(entry.outputPath), { recursive: true });
        const mapName = `${basename(entry.outputPath)}.map`;
        const generatedCode =
          entry.generated.sourceMap === undefined
            ? entry.generated.code
            : `${entry.generated.code}//# sourceMappingURL=${mapName}\n`;
        writeFileSync(entry.outputPath, generatedCode, "utf8");
        if (entry.generated.sourceMap !== undefined) {
          const map = JSON.parse(entry.generated.sourceMap) as Record<string, unknown>;
          map["file"] = basename(entry.outputPath);
          map["sources"] = [
            relative(
              dirname(entry.outputPath),
              entry.moduleSource.absolutePath,
            )
              .split(sep)
              .join("/"),
          ];
          writeFileSync(
            `${entry.outputPath}.map`,
            `${JSON.stringify(map)}\n`,
            "utf8",
          );
        }
        if (entry.generated.declarations !== undefined) {
          writeFileSync(
            entry.outputPath.replace(/\.(js|ts)$/i, ".d.ts"),
            entry.generated.declarations,
            "utf8",
          );
        }
        for (const artifact of entry.generated.artifacts ?? []) {
          const artifactPath = resolve(outDir, artifact.path);
          const artifactRelative = relative(outDir, artifactPath);
          if (
            artifactRelative === ".." ||
            artifactRelative.startsWith(`..${sep}`) ||
            isAbsolute(artifactRelative)
          ) {
            throw new CliError(
              "NS8004",
              `Script artifact '${artifact.path}' escapes the output directory.`,
              displayPath(entry.moduleSource.absolutePath, cwd),
            );
          }
          mkdirSync(dirname(artifactPath), { recursive: true });
          const artifactMapName = `${basename(artifactPath)}.map`;
          const artifactCode =
            artifact.sourceMap === undefined
              ? artifact.code
              : `${artifact.code}//# sourceMappingURL=${artifactMapName}\n`;
          writeFileSync(artifactPath, artifactCode, "utf8");
          if (artifact.sourceMap !== undefined) {
            const map = JSON.parse(artifact.sourceMap) as Record<string, unknown>;
            map["file"] = basename(artifactPath);
            map["sources"] = [
              relative(
                dirname(artifactPath),
                entry.moduleSource.absolutePath,
              )
                .split(sep)
                .join("/"),
            ];
            writeFileSync(
              `${artifactPath}.map`,
              `${JSON.stringify(map)}\n`,
              "utf8",
            );
          }
          if (artifact.declarations !== undefined) {
            writeFileSync(
              artifactPath.replace(/\.(js|ts)$/i, ".d.ts"),
              artifact.declarations,
              "utf8",
            );
          }
        }
      }
      const packagePath = join(outDir, "package.json");
      if (!existsSync(packagePath)) {
        writeFileSync(packagePath, '{ "type": "module" }\n', "utf8");
      }
    } catch (error) {
      throw new CliError(
        "NS9014",
        `Unable to write the generated module: ${errorMessage(error)}`,
        displayPath(outputPath, cwd),
      );
    }

    report = {
      target: targetName,
      backend: String(backend),
      entry: entryKey,
      output: displayPath(outputPath, cwd),
      diagnostics: allDiagnostics,
    };
    if (options.json) {
      printBuildReport(report);
    } else {
      for (const diagnostic of allDiagnostics) {
        printDiagnostic(diagnostic);
      }
      process.stdout.write(`built ${targetName} -> ${displayPath(outputPath, cwd)}\n`);
    }
    return 0;
  } catch (error) {
    const diagnostic = errorDiagnostic(error);
    report = { ...report, diagnostics: [diagnostic] };
    if (options.json) {
      printBuildReport(report);
    } else {
      printDiagnostic(diagnostic);
    }
    return 1;
  }
}

async function runDoctor(args: readonly string[], cwd: string): Promise<number> {
  const directory = args.find((a) => !a.startsWith("--")) ?? ".";
  const root = resolve(cwd, directory);
  const json = args.includes("--json");
  try {

    const buildData = (loadBuildManifest(root)?.data ?? {}) as Record<string, unknown>;
    const policy = parseToolchainPolicy(buildData);
    const probe: ToolchainProbe = {
      exists: (p: string) => existsSync(p),
      run: (cmd: string, a: readonly string[]) => {
        const r = spawnSync(cmd, [...a], { encoding: "utf8", windowsHide: true, input: a.includes("-") ? "#include <stdio.h>\nint main(void){return 0;}\n" : undefined });
        return { status: r.status, stdout: r.stdout ?? "", stderr: r.stderr ?? r.error?.message ?? "" };
      },
      readFileSync: (p: string) => readFileSync(p, "utf8"),
      readdir: (p: string) => readdirSync(p),
    };
    const env = process.env["ASTRA_CC"] ? { ASTRA_CC: process.env["ASTRA_CC"] } : {};
    const noDownload = args.includes("--no-download");
    const result = await doctorAsync(root, policy, env, probe, {
      allowDownload: !noDownload,
    });
    if (json) {
      process.stdout.write(JSON.stringify({ ok: result.winner !== null, doctor: result }, null, 2) + "\n");
    } else {
      process.stdout.write(`policy: ${policy.request}\n`);
      for (const a of result.attempts) {
        process.stdout.write(`  ${a.rung}: ${a.detail}\n`);
      }
      if (result.winner) {
        process.stdout.write(`resolved: ${result.winner.id} (${result.winner.family}) at ${result.winner.path}\n`);
      } else {
        process.stderr.write(`unresolved: ${result.error}\n`);
      }
    }
    return result.winner ? 0 : 1;
  } catch (error) {
    const diagnostic = errorDiagnostic(error);
    if (json) {
      process.stdout.write(JSON.stringify({ ok: false, error: diagnostic }, null, 2) + "\n");
    } else {
      process.stderr.write(`${diagnostic.code}: ${diagnostic.message}\n`);
    }
    return 1;
  }
}

export async function runCli(args: readonly string[], cwd = process.cwd()): Promise<number> {
  const [command, ...rest] = args;

  if (command === undefined || command === "-h" || command === "--help" || command === "help") {
    printHelp();
    return 0;
  }

  if (command === "-v" || command === "--version" || command === "version") {
    printVersion();
    return 0;
  }

  switch (command) {
    case "doctor":
      return await runDoctor(rest, cwd);
    case "check":
      return runCheck(rest, cwd);
    case "fmt":
      return runFmt(rest, cwd);
    case "build":
      return await runBuild(rest, cwd);
    case "ast":
      return runInspection("ast", rest, cwd);
    case "explain":
      return runInspection("explain", rest, cwd);
    default:
      process.stderr.write(`astra: unknown command '${command}'.\n\n`);
      printHelp();
      return 1;
  }
}

const entry = process.argv[1];
const isDirectCliEntry =
  entry !== undefined &&
  /(?:^|[\\/])index\.js$/.test(entry) &&
  import.meta.url === pathToFileURL(entry).href;
if (isDirectCliEntry) {
  void runCli(process.argv.slice(2)).then(
    (code) => {
      process.exitCode = code;
    },
    (error: unknown) => {
      process.stderr.write(
        `${error instanceof Error ? error.message : String(error)}\n`,
      );
      process.exitCode = 1;
    },
  );
}
