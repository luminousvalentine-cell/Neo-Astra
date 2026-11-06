import { readFileSync } from "node:fs";
import { resolve } from "node:path";

import { loadManifest, loadBuildManifest, type ManifestDocument } from "@neo-astra/manifest";

import {
  NopmError,
  type DependencyRequest,
  type Mode,
  type PackageId,
  type TargetSpec,
  formatPackageId,
  hashManifestSource,
  parseDependencyRequest,
} from "./index.js";

export interface ProjectDependency {
  readonly alias: string;
  readonly request: DependencyRequest;
  readonly targets: readonly string[];
}

export interface ProjectManifest {
  readonly directory: string;
  readonly file: ManifestDocument["file"];
  readonly source: string;
  readonly manifestHash: string;
  readonly packageName: string | null;
  readonly packageVersion: string | null;
  readonly dependencies: readonly ProjectDependency[];
  readonly targets: readonly TargetSpec[];
  readonly registry: string | null;
  readonly defaultSource: string;
  readonly packageSources: Readonly<Record<string, boolean>>;
  readonly capabilities: readonly string[];
  readonly ffi: Readonly<Record<string, unknown>>;
}

export function readProject(
  directory: string,
  options: { readonly requireTargets?: boolean } = {},
): ProjectManifest {
  const absolute = resolve(directory);
  const manifest = loadManifest(absolute);
  const buildManifest = loadBuildManifest(absolute);
  const source = readFileSync(manifest.file.path, "utf8");
  const data = { ...manifest.data, ...(buildManifest?.data ?? {}) };
  const packageRecord = recordValue(data, "package");
  const registryRecord = recordValue(data, "registry");
  const capabilitiesRecord = recordValue(data, "capabilities");

  return {
    directory: absolute,
    file: manifest.file,
    source,
    manifestHash: hashManifestSource(source),
    packageName: stringValue(packageRecord, "name") ?? null,
    packageVersion: stringValue(packageRecord, "version") ?? null,
    dependencies: readDependencies(data, registryRecord),
    targets: readTargets(data, options.requireTargets ?? true),
    registry: stringValue(registryRecord, "default") ?? null,
    defaultSource: stringValue(registryRecord, "defaultSource") ?? "nova",
    packageSources: readPackageSources(data),
    capabilities: collectCapabilities(capabilitiesRecord),
    ffi: recordValue(data, "ffi"),
  };
}

export function packageDirectory(projectRoot: string, id: PackageId): string {
  if (id.source !== "path" || id.path === null) {
    throw new NopmError("NP0011", "Expected a path package ID.", formatPackageId(id));
  }

  const target = resolve(projectRoot, id.path);
  const normalizedRoot = projectRoot.split("\\").join("/").replace(/\/+$/, "");
  const normalizedTarget = target.split("\\").join("/");
  if (
    normalizedTarget !== normalizedRoot &&
    !normalizedTarget.startsWith(`${normalizedRoot}/`)
  ) {
    throw new NopmError(
      "NP0011",
      `Path package '${id.path}' escapes the project root.`,
      id.path,
    );
  }
  return target;
}

function readDependencies(
  data: Record<string, unknown>,
  registry: Record<string, unknown>,
): readonly ProjectDependency[] {
  const raw = recordValue(data, "dependencies");
  const defaultSource = stringValue(registry, "defaultSource") ?? "nova";
  const dependencies: ProjectDependency[] = [];

  for (const alias of Object.keys(raw).sort()) {
    const declaration = raw[alias];
    const request = dependencyRequest(alias, declaration, defaultSource);
    const targets =
      isRecord(declaration) && Array.isArray(declaration["targets"])
        ? stringArray(declaration["targets"], `dependency '${alias}' targets`)
        : [];
    dependencies.push(Object.freeze({ alias, request, targets: Object.freeze(targets) }));
  }

  return Object.freeze(dependencies);
}

function dependencyRequest(
  alias: string,
  declaration: unknown,
  defaultSource: string,
): DependencyRequest {
  if (typeof declaration === "string") {
    return parseDependencyRequest(alias, declaration);
  }
  if (!isRecord(declaration)) {
    throw new NopmError(
      "NP0002",
      `Dependency '${alias}' must be a source-qualified string or an object.`,
      alias,
    );
  }

  const source = stringValue(declaration, "source") ?? defaultSource;
  const name = stringValue(declaration, "name") ?? alias;
  const version = stringValue(declaration, "version");
  const path = stringValue(declaration, "path");
  const url = stringValue(declaration, "url");
  const reference = stringValue(declaration, "ref") ?? stringValue(declaration, "reference");

  let spec: string;
  switch (source) {
    case "path":
      if (path === null) {
        throw new NopmError("NP0002", `Path dependency '${alias}' requires a path.`, alias);
      }
      spec = `path:${path}`;
      break;
    case "git":
      if (url === null || reference === null) {
        throw new NopmError(
          "NP0002",
          `Git dependency '${alias}' requires url and ref.`,
          alias,
        );
      }
      spec = `git+${url}#${reference}`;
      break;
    case "nova":
    case "npm":
    case "cargo":
    case "pypi":
    case "nuget":
      spec = `${source}:${name}${version === null ? "" : `@${version}`}`;
      break;
    case "oci":
      if (url === null || reference === null) {
        throw new NopmError(
          "NP0002",
          `OCI dependency '${alias}' requires url and reference.`,
          alias,
        );
      }
      spec = `oci:${url}@${reference}`;
      break;
    default:
      throw new NopmError(
        "NP0002",
        `Dependency '${alias}' uses unsupported source '${source}'.`,
        alias,
      );
  }

  return parseDependencyRequest(alias, spec);
}

function readTargets(
  data: Record<string, unknown>,
  requireTargets: boolean,
): readonly TargetSpec[] {
  const raw = recordValue(data, "targets");
  const build = recordValue(data, "build");
  const defaultMode = stringValue(build, "defaultMode");
  const targets: TargetSpec[] = [];

  for (const name of Object.keys(raw).sort()) {
    const declaration = raw[name];
    if (!isRecord(declaration)) {
      throw new NopmError("NP0006", `Target '${name}' must be an object.`, name);
    }
    const backend = stringValue(declaration, "backend");
    if (backend === null) {
      throw new NopmError("NP0006", `Target '${name}' requires a backend.`, name);
    }
    const mode = stringValue(declaration, "mode") ?? defaultMode ?? defaultModeFor(name, backend);
    if (!isMode(mode)) {
      throw new NopmError("NP0007", `Target '${name}' has invalid mode '${mode}'.`, mode);
    }
    const triple = stringValue(declaration, "triple");
    targets.push(
      Object.freeze({
        name,
        backend,
        mode,
        ...(triple === null ? {} : { triple }),
      }),
    );
  }

  if (targets.length === 0) {
    const defaultTarget = stringValue(build, "defaultTarget");
    if (defaultTarget !== null) {
      targets.push(
        Object.freeze({
          name: defaultTarget,
          backend: "js",
          mode: isMode(defaultMode ?? "") ? (defaultMode as Mode) : "web",
        }),
      );
    }
  }

  if (targets.length === 0 && requireTargets) {
    throw new NopmError(
      "NP0006",
      "No project targets are configured. Add a [targets] entry to the manifest.",
      "targets",
    );
  }

  return Object.freeze(targets);
}

function readPackageSources(data: Record<string, unknown>): Readonly<Record<string, boolean>> {
  const raw = recordValue(data, "packageSources");
  const sources: Record<string, boolean> = {};
  for (const key of Object.keys(raw).sort()) {
    const value = raw[key];
    if (typeof value === "boolean") {
      sources[key] = value;
    } else if (isRecord(value)) {
      sources[key] = value["enabled"] === true;
    }
  }
  return Object.freeze(sources);
}

function collectCapabilities(raw: Record<string, unknown>): readonly string[] {
  const capabilities = new Set<string>();
  for (const value of Object.values(raw)) {
    if (Array.isArray(value)) {
      for (const entry of value) {
        if (typeof entry === "string") {
          capabilities.add(entry);
        }
      }
    }
  }
  return Object.freeze([...capabilities].sort());
}

function defaultModeFor(name: string, backend: string): Mode {
  if (backend === "js" || backend === "ts" || name.includes("web")) {
    return "web";
  }
  if (backend === "c" || backend === "wasm32" || name.includes("native") || name.includes("wasi")) {
    return "system";
  }
  return "core";
}

function isMode(value: string): value is Mode {
  return value === "core" || value === "app" || value === "web" || value === "system";
}

function recordValue(
  value: Record<string, unknown>,
  key: string,
): Record<string, unknown> {
  const entry = value[key];
  return isRecord(entry) ? entry : {};
}

function stringValue(
  value: Record<string, unknown>,
  key: string,
): string | null {
  const entry = value[key];
  return typeof entry === "string" ? entry : null;
}

function stringArray(value: unknown, label: string): string[] {
  if (!Array.isArray(value) || value.some((entry) => typeof entry !== "string")) {
    throw new NopmError("NP0002", `${label} must be an array of strings.`, label);
  }
  return [...value] as string[];
}

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}
