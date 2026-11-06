import { existsSync, readFileSync, writeFileSync } from "node:fs";
import { basename, join } from "node:path";
import { parse as parseToml } from "smol-toml";
import * as taploImport from "@taplo/lib";
import type { Taplo as TaploInstance } from "@taplo/lib";

export const CORE_MANIFEST_NAMES = ["core.json", "core.toml"] as const;
export const BUILD_MANIFEST_NAMES = ["build.json", "build.toml"] as const;
export const MANIFEST_NAMES = CORE_MANIFEST_NAMES;

export const CORE_KEYS = [
  "package",
  "dependencies",
  "packageSources",
  "registry",
  "ffi",
  "capabilities",
] as const;

export const BUILD_KEYS = [
  "build",
  "targets",
  "nova",
  "math",
  "strings",
  "memory",
  "toolchain",
] as const;

export function isCoreManifestName(name: string): boolean {
  return (CORE_MANIFEST_NAMES as readonly string[]).includes(name);
}

export function isBuildManifestName(name: string): boolean {
  return (BUILD_MANIFEST_NAMES as readonly string[]).includes(name);
}

export type ManifestFormat = "json" | "toml";

export interface ManifestFile {
  readonly path: string;
  readonly name: string;
  readonly format: ManifestFormat;
  readonly kind: "core" | "build";
}

export interface ManifestDocument {
  readonly file: ManifestFile;
  readonly data: Record<string, unknown>;
}

export type ManifestErrorCode = "NS9001" | "NS9002" | "NS9003";

export class ManifestError extends Error {
  readonly code: ManifestErrorCode;
  readonly path: string | undefined;

  constructor(code: ManifestErrorCode, message: string, path?: string) {
    super(message);
    this.name = "ManifestError";
    this.code = code;
    this.path = path;
  }
}

const taploModule = taploImport as unknown as {
  readonly Taplo?: { initialize: () => Promise<TaploInstance> };
  readonly default?: { Taplo?: { initialize: () => Promise<TaploInstance> } };
};
function resolveTaploCompiler(): {
  initialize: () => Promise<TaploInstance>;
} {
  const compiler = taploModule.Taplo ?? taploModule.default?.Taplo;
  if (compiler === undefined) {
    throw new Error("@taplo/lib did not expose Taplo");
  }
  return compiler;
}
const TaploCompiler = resolveTaploCompiler();
let taploPromise: Promise<TaploInstance> | undefined;

const CORE_ROOT_ORDER = [
  "$schema",
  "package",
  "dependencies",
  "packageSources",
  "registry",
  "ffi",
  "capabilities",
] as const;

const BUILD_ROOT_ORDER = [
  "$schema",
  "build",
  "targets",
  "nova",
  "math",
  "strings",
  "memory",
  "toolchain",
] as const;

const KEY_ORDERS: Readonly<Record<string, readonly string[]>> = {
  package: [
    "name",
    "version",
    "edition",
    "license",
    "description",
    "authors",
    "repository",
    "homepage",
    "keywords",
  ],
  build: ["defaultTarget", "defaultMode", "profile", "outDir"],
  nova: ["entry", "watch"],
  registry: ["default", "defaultSource", "mirrors"],
  math: ["integers"],
  strings: ["model"],
  memory: ["default", "files"],
  target: ["backend", "module", "triple", "mode", "outDir", "wasi", "env"],
  dependency: [
    "source",
    "version",
    "path",
    "git",
    "ref",
    "rev",
    "tag",
    "targets",
    "features",
    "optional",
  ],
  packageSource: ["enabled", "lifecycleScripts", "registry", "index"],
  packageSources: ["path", "git", "npm", "cargo", "pypi", "nuget", "oci"],
  ffiTarget: ["source", "type", "ownership"],
  capability: ["web", "native", "wasm"],
};

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}

function sortedKeys(value: Record<string, unknown>): readonly string[] {
  return Object.keys(value).sort((left, right) => left.localeCompare(right));
}

function orderedKeys(
  value: Record<string, unknown>,
  order: readonly string[],
): readonly string[] {
  const present = order.filter((key) => Object.hasOwn(value, key));
  const remaining = sortedKeys(value).filter((key) => !order.includes(key));
  return [...present, ...remaining];
}

function orderRecord(
  value: Record<string, unknown>,
  order: readonly string[],
): Record<string, unknown> {
  const result: Record<string, unknown> = {};

  for (const key of orderedKeys(value, order)) {
    result[key] = orderValue(value[key], key);
  }

  return result;
}

function orderMap(
  value: Record<string, unknown>,
  order: readonly string[],
): Record<string, unknown> {
  const result: Record<string, unknown> = {};

  for (const key of sortedKeys(value)) {
    const entry = value[key];
    result[key] = isRecord(entry) ? orderRecord(entry, order) : orderValue(entry, key);
  }

  return result;
}

function orderKnownMap(
  value: Record<string, unknown>,
  outerOrder: readonly string[],
  childOrder: readonly string[],
): Record<string, unknown> {
  const result: Record<string, unknown> = {};

  for (const key of orderedKeys(value, outerOrder)) {
    const entry = value[key];
    result[key] = isRecord(entry)
      ? orderRecord(entry, childOrder)
      : orderValue(entry, key);
  }

  return result;
}
function orderValue(value: unknown, key: string | undefined): unknown {
  if (Array.isArray(value)) {
    return value.map((entry) => orderValue(entry, undefined));
  }

  if (!isRecord(value)) {
    return value;
  }

  switch (key) {
    case undefined:
    case "$schema":
      return orderRecord(value, []);
    case "targets":
      return orderMap(value, KEY_ORDERS["target"] ?? []);
    case "dependencies":
      return orderMap(value, KEY_ORDERS["dependency"] ?? []);
    case "packageSources":
      return orderKnownMap(
        value,
        KEY_ORDERS["packageSources"] ?? [],
        KEY_ORDERS["packageSource"] ?? [],
      );
    case "ffi": {
      const result: Record<string, unknown> = {};
      for (const moduleName of Object.keys(value)) {
        const moduleValue = value[moduleName];
        result[moduleName] = isRecord(moduleValue)
          ? orderRecord(moduleValue, KEY_ORDERS["ffiTarget"] ?? [])
          : moduleValue;
      }
      return result;
    }
    case "capabilities":
      return orderRecord(value, KEY_ORDERS["capability"] ?? []);
    case "memory":
    case "package":
    case "build":
    case "nova":
    case "registry":
    case "math":
    case "strings":
    case "toolchain":
      return orderRecord(value, KEY_ORDERS[key] ?? []);
    default:
      return orderRecord(value, []);
  }
}

function orderManifest(data: Record<string, unknown>, kind: "core" | "build" = "core"): Record<string, unknown> {
  const order = kind === "build" ? BUILD_ROOT_ORDER : CORE_ROOT_ORDER;
  return orderRecord(data, order);
}

function getTaplo(): Promise<TaploInstance> {
  taploPromise ??= TaploCompiler.initialize();
  return taploPromise;
}

export function findCoreManifest(cwd = process.cwd()): ManifestFile {
  const matches = CORE_MANIFEST_NAMES.filter((name) => existsSync(join(cwd, name)));

  if (matches.length === 0) {
    throw new ManifestError(
      "NS9001",
      "No manifest found. Expected exactly one of core.json or core.toml.",
      cwd,
    );
  }

  if (matches.length > 1) {
    throw new ManifestError(
      "NS9001",
      "Both core.json and core.toml exist. Exactly one manifest is allowed.",
      cwd,
    );
  }

  const name = matches[0];
  if (name === undefined) {
    throw new ManifestError("NS9001", "Manifest discovery failed.", cwd);
  }

  return {
    path: join(cwd, name),
    name,
    format: name.endsWith(".json") ? "json" : "toml",
    kind: "core",
  };
}

export function findBuildManifest(cwd = process.cwd()): ManifestFile | undefined {
  const matches = BUILD_MANIFEST_NAMES.filter((name) => existsSync(join(cwd, name)));

  if (matches.length > 1) {
    throw new ManifestError(
      "NS9001",
      "Both build.json and build.toml exist. At most one is allowed.",
      cwd,
    );
  }

  const name = matches[0];
  if (name === undefined) {
    return undefined;
  }

  return {
    path: join(cwd, name),
    name,
    format: name.endsWith(".json") ? "json" : "toml",
    kind: "build",
  };
}

export function findManifest(cwd = process.cwd()): ManifestFile {
  return findCoreManifest(cwd);
}

export function findManifests(cwd = process.cwd()): {
  core: ManifestFile;
  build: ManifestFile | undefined;
} {
  return { core: findCoreManifest(cwd), build: findBuildManifest(cwd) };
}

export function manifestFileForPath(path: string): ManifestFile | undefined {
  const name = basename(path);
  const core = isCoreManifestName(name);
  const build = isBuildManifestName(name);
  if (!core && !build) {
    return undefined;
  }

  return {
    path,
    name,
    format: name.endsWith(".json") ? "json" : "toml",
    kind: core ? "core" : "build",
  };
}

function assertNoDuplicateJsonKeys(source: string, file: ManifestFile): void {
  let index = 0;

  const skipWhitespace = (): void => {
    while (index < source.length && /\s/.test(source[index] ?? "")) {
      index += 1;
    }
  };

  const parseString = (): string => {
    const start = index;
    index += 1;

    while (index < source.length) {
      const character = source[index];
      if (character === "\\") {
        index += 2;
        continue;
      }
      if (character === '"') {
        index += 1;
        return JSON.parse(source.slice(start, index)) as string;
      }
      index += 1;
    }

    throw new ManifestError("NS9002", "Unterminated JSON string.", file.path);
  };

  const parseValue = (): void => {
    skipWhitespace();
    const character = source[index];

    if (character === "{") {
      parseObject();
      return;
    }

    if (character === "[") {
      parseArray();
      return;
    }

    if (character === '"') {
      parseString();
      return;
    }

    while (
      index < source.length &&
      !/[\s,\]}]/.test(source[index] ?? "")
    ) {
      index += 1;
    }
  };

  const parseObject = (): void => {
    index += 1;
    skipWhitespace();
    const keys = new Set<string>();

    if (source[index] === "}") {
      index += 1;
      return;
    }

    while (index < source.length) {
      skipWhitespace();
      if (source[index] !== '"') {
        throw new ManifestError("NS9002", "Invalid JSON object key.", file.path);
      }

      const key = parseString();
      if (keys.has(key)) {
        throw new ManifestError(
          "NS9002",
          `Duplicate key '${key}' in JSON manifest.`,
          file.path,
        );
      }
      keys.add(key);

      skipWhitespace();
      if (source[index] !== ":") {
        throw new ManifestError("NS9002", "Invalid JSON object entry.", file.path);
      }
      index += 1;
      parseValue();
      skipWhitespace();

      if (source[index] === ",") {
        index += 1;
        continue;
      }

      if (source[index] === "}") {
        index += 1;
        return;
      }

      throw new ManifestError("NS9002", "Invalid JSON object entry.", file.path);
    }

    throw new ManifestError("NS9002", "Unterminated JSON object.", file.path);
  };

  const parseArray = (): void => {
    index += 1;
    skipWhitespace();

    if (source[index] === "]") {
      index += 1;
      return;
    }

    while (index < source.length) {
      parseValue();
      skipWhitespace();

      if (source[index] === ",") {
        index += 1;
        continue;
      }

      if (source[index] === "]") {
        index += 1;
        return;
      }

      throw new ManifestError("NS9002", "Invalid JSON array entry.", file.path);
    }

    throw new ManifestError("NS9002", "Unterminated JSON array.", file.path);
  };

  parseValue();
  skipWhitespace();

  if (index !== source.length) {
    throw new ManifestError("NS9002", "Invalid JSON trailing content.", file.path);
  }
}

export function parseManifestSource(
  source: string,
  file: ManifestFile,
): Record<string, unknown> {
  let value: unknown;
  const normalizedSource = source.replace(/^\uFEFF/, "");

  try {
    if (file.format === "json") {
      value = JSON.parse(normalizedSource);
      assertNoDuplicateJsonKeys(normalizedSource, file);
    } else {
      value = parseToml(normalizedSource);
    }
  } catch (error) {
    if (error instanceof ManifestError) {
      throw error;
    }

    throw new ManifestError(
      "NS9002",
      `Invalid ${file.format.toUpperCase()} manifest: ${error instanceof Error ? error.message : String(error)}`,
      file.path,
    );
  }

  if (!isRecord(value)) {
    throw new ManifestError("NS9002", "Manifest root must be an object.", file.path);
  }

  return value;
}

export function validateManifestKeys(file: ManifestFile, data: Record<string, unknown>): void {
  const coreKeySet = new Set(CORE_KEYS as readonly string[]);
  const buildKeySet = new Set(BUILD_KEYS as readonly string[]);

  for (const key of Object.keys(data)) {
    if (file.kind === "core" && buildKeySet.has(key)) {
      throw new ManifestError(
        "NS9002",
        `Key '${key}' belongs in build.${file.format === "json" ? "json" : "toml"}, not ${file.name}.`,
        file.path,
      );
    }
    if (file.kind === "build" && coreKeySet.has(key)) {
      throw new ManifestError(
        "NS9002",
        `Key '${key}' belongs in core.${file.format === "json" ? "json" : "toml"}, not ${file.name}.`,
        file.path,
      );
    }
  }
}

export function readManifest(file: ManifestFile): Record<string, unknown> {
  let source: string;

  try {
    source = readFileSync(file.path, "utf8");
  } catch (error) {
    throw new ManifestError(
      "NS9002",
      `Unable to read manifest: ${error instanceof Error ? error.message : String(error)}`,
      file.path,
    );
  }

  return parseManifestSource(source, file);
}

export async function formatManifestSource(
  source: string,
  file: ManifestFile,
): Promise<string> {
  const data = parseManifestSource(source, file);

  if (file.format === "json") {
    return `${JSON.stringify(orderManifest(data, file.kind), null, 2)}\n`;
  }

  const taplo = await getTaplo();
  let formatted: string;

  try {
    formatted = taplo.format(source.replace(/^\uFEFF/, ""));
  } catch (error) {
    throw new ManifestError(
      "NS9002",
      `Invalid TOML manifest: ${error instanceof Error ? error.message : String(error)}`,
      file.path,
    );
  }

  return `${formatted.replace(/\r\n/g, "\n").trimEnd()}\n`;
}

export function loadCoreManifest(cwd = process.cwd()): ManifestDocument {
  const file = findCoreManifest(cwd);
  const data = readManifest(file);
  validateManifestKeys(file, data);
  return { file, data };
}

export function loadBuildManifest(cwd = process.cwd()): ManifestDocument | undefined {
  const file = findBuildManifest(cwd);
  if (file === undefined) {
    return undefined;
  }
  const data = readManifest(file);
  validateManifestKeys(file, data);
  return { file, data };
}

export function loadManifest(cwd = process.cwd()): ManifestDocument {
  return loadCoreManifest(cwd);
}

export function loadProjectManifests(cwd = process.cwd()): {
  core: ManifestDocument;
  build: ManifestDocument | undefined;
} {
  return { core: loadCoreManifest(cwd), build: loadBuildManifest(cwd) };
}

export const BUILD_DEFAULTS = {
  build: { defaultTarget: "native" },
  targets: { native: { backend: "native", outDir: "dist/native" } },
  nova: { entry: "src/main.ns" },
  math: { integers: "wrapping" },
  strings: {},
  memory: {},
  toolchain: { c: "auto" },
} as const;

export function applyBuildDefaults(
  data: Record<string, unknown> | undefined,
): Record<string, unknown> {
  const result: Record<string, unknown> = { ...BUILD_DEFAULTS };
  if (data !== undefined) {
    for (const [key, value] of Object.entries(data)) {
      result[key] = value;
    }
  }
  return result;
}

export function getPackageName(manifest: ManifestDocument): string | undefined {
  const pkg = manifest.data["package"];
  if (!isRecord(pkg)) {
    return undefined;
  }
  const name = pkg["name"];
  return typeof name === "string" ? name : undefined;
}

export async function addDependency(
  file: ManifestFile,
  name: string,
  spec: string,
): Promise<void> {
  const source = readFileSync(file.path, "utf8");
  const data = parseManifestSource(source, file);
  const deps = isRecord(data["dependencies"]) ? { ...data["dependencies"] } : {};

  if (Object.hasOwn(deps, name)) {
    throw new ManifestError(
      "NS9003",
      `Dependency '${name}' already exists in ${file.name}.`,
      file.path,
    );
  }

  deps[name] = spec;
  data["dependencies"] = deps;

  let formatted: string;
  if (file.format === "json") {
    formatted = `${JSON.stringify(orderManifest(data, file.kind), null, 2)}\n`;
  } else {

    const headerRe = /^\s*\[dependencies\]\s*(#.*)?$/;
    let updated = source;
    const lines = updated.split("\n");
    const headerIndex = lines.findIndex((line) => headerRe.test(line));
    if (headerIndex >= 0) {
      lines.splice(headerIndex + 1, 0, `${name} = ${JSON.stringify(spec)}`);
      updated = lines.join("\n");
    } else {
      updated = `${updated.trimEnd()}\n\n[dependencies]\n${name} = ${JSON.stringify(spec)}\n`;
    }
    const taplo = await getTaplo();
    try {
      formatted = taplo.format(updated.replace(/^\uFEFF/, ""));
    } catch (error) {
      throw new ManifestError(
        "NS9002",
        `Invalid TOML manifest: ${error instanceof Error ? error.message : String(error)}`,
        file.path,
      );
    }
    formatted = `${formatted.replace(/\r\n/g, "\n").trimEnd()}\n`;
  }

  writeFileSync(file.path, formatted, "utf8");
}

export async function removeDependency(file: ManifestFile, name: string): Promise<boolean> {
  const source = readFileSync(file.path, "utf8");
  const data = parseManifestSource(source, file);
  const deps = isRecord(data["dependencies"]) ? { ...data["dependencies"] } : {};

  if (!Object.hasOwn(deps, name)) {
    return false;
  }

  delete deps[name];

  if (Object.keys(deps).length === 0) {
    delete data["dependencies"];
  } else {
    data["dependencies"] = deps;
  }

  let formatted: string;
  if (file.format === "json") {
    formatted = `${JSON.stringify(orderManifest(data, file.kind), null, 2)}\n`;
  } else {
    const headerRe = /^\s*\[dependencies\]\s*(#.*)?$/;
    const lines = source.split("\n");
    let inDeps = false;
    const result: string[] = [];
    for (const line of lines) {
      if (/^\s*\[/.test(line)) {
        inDeps = headerRe.test(line);
        result.push(line);
        continue;
      }
      if (inDeps) {
        const m = line.match(/^\s*([A-Za-z0-9_.-]+)\s*=/);
        if (m && m[1] === name) {
          continue;
        }
      }
      result.push(line);
    }

    const cleaned: string[] = [];
    let i = 0;
    while (i < result.length) {
      if (headerRe.test(result[i] ?? "")) {
        let j = i + 1;
        while (j < result.length && (result[j]?.trim() ?? "") === "") j++;
        if (j >= result.length || /^\s*\[/.test(result[j] ?? "")) {
          i++;
          while (i < result.length && (result[i]?.trim() ?? "") === "") i++;
          continue;
        }
      }
      cleaned.push(result[i] ?? "");
      i++;
    }
    let updated = cleaned.join("\n");
    const taplo = await getTaplo();
    try {
      formatted = taplo.format(updated.replace(/^\uFEFF/, ""));
    } catch (error) {
      throw new ManifestError(
        "NS9002",
        `Invalid TOML manifest: ${error instanceof Error ? error.message : String(error)}`,
        file.path,
      );
    }
    formatted = `${formatted.replace(/\r\n/g, "\n").trimEnd()}\n`;
  }

  writeFileSync(file.path, formatted, "utf8");
  return true;
}
