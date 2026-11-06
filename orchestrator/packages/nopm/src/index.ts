import { createHash } from "node:crypto";
import { readFileSync, renameSync, rmSync, writeFileSync } from "node:fs";

export type PackageSource =
  | "nova"
  | "npm"
  | "cargo"
  | "git"
  | "path"
  | "pypi"
  | "nuget"
  | "oci";

export type NopmErrorCode =
  | "NP0001"
  | "NP0002"
  | "NP0003"
  | "NP0004"
  | "NP0005"
  | "NP0006"
  | "NP0007"
  | "NP0008"
  | "NP0009"
  | "NP0010"
  | "NP0011"
  | "NP0012"
  | "NP0013"
  | "NP0014"
  | "NP0015"
  | "NP0016"
  | "NP0017"
  | "NP0018";

export type Mode = "core" | "app" | "web" | "system";

export interface TargetSpec {
  readonly name: string;
  readonly backend: string;
  readonly mode: Mode;
  readonly triple?: string;
}

export class NopmError extends Error {
  readonly code: NopmErrorCode;
  readonly input: string | undefined;

  constructor(code: NopmErrorCode, message: string, input?: string, options?: ErrorOptions) {
    super(message, options);
    this.name = "NopmError";
    this.code = code;
    this.input = input;
  }
}

export interface PackageId {
  readonly source: PackageSource;
  readonly name: string;
  readonly version: string | null;
  readonly reference: string | null;
  readonly path: string | null;
  readonly raw: string;
}

export interface DependencyRequest {
  readonly alias: string;
  readonly spec: string;
  readonly id: PackageId;
}

export interface PackageDescriptor {
  readonly id: PackageId;
  readonly dependencies: Readonly<Record<string, string>>;
  readonly targets?: readonly string[];
  readonly capabilities?: readonly string[];
  readonly integrity?: string;
  readonly resolved?: string;
  readonly registry?: string;
  readonly commit?: string;
  readonly lifecycleScripts?: boolean;
  readonly metadata?: Readonly<Record<string, unknown>>;
}

export interface DescriptorInput {
  readonly id: PackageId;
  readonly dependencies?: Readonly<Record<string, string>>;
  readonly targets?: readonly string[];
  readonly capabilities?: readonly string[];
  readonly integrity?: string;
  readonly resolved?: string;
  readonly registry?: string;
  readonly commit?: string;
  readonly lifecycleScripts?: boolean;
  readonly metadata?: Readonly<Record<string, unknown>>;
}

export interface DependencyGraphNode {
  readonly id: PackageId;
  readonly descriptor: PackageDescriptor;
  readonly dependencies: readonly DependencyGraphNode[];
}

export interface DependencyGraph {
  readonly roots: readonly DependencyGraphNode[];
  readonly nodes: readonly DependencyGraphNode[];
}

export interface ResolvedTarget {
  readonly target: TargetSpec;
  readonly graph: DependencyGraph;
}

export interface ProjectResolution {
  readonly targets: readonly ResolvedTarget[];
  readonly packages: readonly LockfilePackage[];
}

export interface LockfileCompiler {
  readonly version: string;
  readonly apiVersion: string;
}

export interface LockfileToolchain {
  readonly rung: "env" | "cache" | "path";
  readonly id: string;
  readonly family: string;
  readonly path: string;
  readonly version: string | null;
}
export interface LockfileSemantics {
  readonly integers: string;
  readonly strings: string;
}

export interface LockfileTarget {
  readonly backend: string;
  readonly mode: Mode;
  readonly triple: string | null;
}

export interface LockfilePackage {
  readonly id: string;
  readonly name: string;
  readonly source: PackageSource;
  readonly version: string | null;
  readonly resolved: string | null;
  readonly registry: string | null;
  readonly integrity: string | null;
  readonly targets: readonly string[];
  readonly capabilities: readonly string[];
  readonly lifecycleScripts: boolean;
  readonly dependencies: readonly string[];
  readonly commit: string | null;
}

export interface Lockfile {
  readonly lockfileVersion: 1;
  readonly generatedBy: string;
  readonly manifestHash: string;
  readonly compiler: LockfileCompiler;
  readonly semantics: LockfileSemantics;
  readonly toolchain: LockfileToolchain | null;
  readonly targets: Readonly<Record<string, LockfileTarget>>;
  readonly packages: readonly LockfilePackage[];
  readonly ffi: Readonly<Record<string, unknown>>;
}

export interface LockfileInput {
  readonly resolution: ProjectResolution;
  readonly generatedBy: string;
  readonly manifestHash: string;
  readonly compiler: LockfileCompiler;
  readonly semantics?: LockfileSemantics;
  readonly toolchain?: LockfileToolchain | null;
  readonly ffi?: Readonly<Record<string, unknown>>;
}

const SEMVER =
  /^(0|[1-9]\d*)\.(0|[1-9]\d*)\.(0|[1-9]\d*)(?:-[0-9A-Za-z-]+(?:\.[0-9A-Za-z-]+)*)?(?:\+[0-9A-Za-z-]+(?:\.[0-9A-Za-z-]+)*)?$/;

const NPM_NAME = /^[A-Za-z0-9._~-]+$/;
const SCOPED_NAME = /^@[A-Za-z0-9._~-]+\/[A-Za-z0-9._~-]+$/;
const CARGO_NAME = /^[A-Za-z0-9_-]+$/;
const ECOSYSTEM_NAME = /^[A-Za-z0-9._-]+$/;
const ALIAS = /^[A-Za-z_][A-Za-z0-9_-]*$/;

const MODE_ORDER: Readonly<Record<Mode, readonly Mode[]>> = {
  core: ["core"],
  app: ["core", "app"],
  web: ["core", "app", "web"],
  system: ["core", "app", "system"],
};

const CAPABILITY_MODES: Readonly<Record<string, Mode>> = {
  console: "app",
  args: "app",
  time: "app",
  env: "app",
  dom: "web",
  fetch: "web",
  worker: "web",
  storage: "web",
  websocket: "web",
  fs: "system",
  net: "system",
  process: "system",
  ffi: "system",
  thread: "system",
  ptr: "system",
  memory: "system",
  os: "system",
};

const DEFAULT_SEMANTICS: LockfileSemantics = Object.freeze({
  integers: "wrapping",
  strings: "utf8",
});

export function parsePackageId(input: string): PackageId {
  if (input.length === 0 || input.trim() !== input) {
    throw invalidId(input, "Package IDs must not be empty or contain surrounding whitespace.");
  }

  if (input.startsWith("nova:")) {
    return parseNova(input, input.slice("nova:".length));
  }
  if (input.startsWith("npm:")) {
    return parseNpm(input, input.slice("npm:".length));
  }
  if (input.startsWith("cargo:")) {
    return parseCargo(input, input.slice("cargo:".length));
  }
  if (input.startsWith("git+")) {
    return parseGit(input);
  }
  if (input.startsWith("path:")) {
    return parsePath(input, input.slice("path:".length));
  }
  if (input.startsWith("pypi:")) {
    return parseEcosystem(input, input.slice("pypi:".length), "pypi");
  }
  if (input.startsWith("nuget:")) {
    return parseEcosystem(input, input.slice("nuget:".length), "nuget");
  }
  if (input.startsWith("oci:")) {
    return parseOci(input, input.slice("oci:".length));
  }

  throw invalidId(
    input,
    "Package IDs must be source-qualified with nova:, npm:, cargo:, git+, path:, pypi:, nuget:, or oci:.",
  );
}

export function formatPackageId(id: PackageId): string {
  switch (id.source) {
    case "nova":
      return `nova:${id.name}@${id.version ?? ""}`;
    case "npm":
      return `npm:${id.name}${id.version === null ? "" : `@${id.version}`}`;
    case "cargo":
      return `cargo:${id.name}${id.version === null ? "" : `@${id.version}`}`;
    case "git":
      return `git+${id.name}#${id.reference ?? ""}`;
    case "path":
      return `path:${id.path ?? ""}`;
    case "pypi":
      return `pypi:${id.name}${id.version === null ? "" : `@${id.version}`}`;
    case "nuget":
      return `nuget:${id.name}${id.version === null ? "" : `@${id.version}`}`;
    case "oci":
      return `oci:${id.name}@${id.reference ?? ""}`;
  }
}

export function parseDependencyRequest(alias: string, spec: string): DependencyRequest {
  if (!ALIAS.test(alias)) {
    throw new NopmError(
      "NP0002",
      `Invalid dependency alias '${alias}'. Use letters, digits, '_' or '-', starting with a letter or '_'.`,
      alias,
    );
  }

  let id: PackageId;
  try {
    id = parsePackageId(spec);
  } catch (error) {
    if (error instanceof NopmError) {
      throw new NopmError(
        "NP0002",
        `Invalid dependency '${alias}': ${error.message}`,
        spec,
        { cause: error },
      );
    }
    throw error;
  }

  return Object.freeze({ alias, spec, id });
}

export function createDescriptor(input: DescriptorInput): PackageDescriptor {
  const dependencies = Object.freeze({ ...(input.dependencies ?? {}) });
  const descriptor: PackageDescriptor = {
    id: input.id,
    dependencies,
    ...(input.targets === undefined ? {} : { targets: Object.freeze([...input.targets]) }),
    ...(input.capabilities === undefined
      ? {}
      : { capabilities: Object.freeze([...input.capabilities]) }),
    ...(input.integrity === undefined ? {} : { integrity: input.integrity }),
    ...(input.resolved === undefined ? {} : { resolved: input.resolved }),
    ...(input.registry === undefined ? {} : { registry: input.registry }),
    ...(input.commit === undefined ? {} : { commit: input.commit }),
    ...(input.lifecycleScripts === undefined
      ? {}
      : { lifecycleScripts: input.lifecycleScripts }),
    ...(input.metadata === undefined ? {} : { metadata: Object.freeze({ ...input.metadata }) }),
  };
  return Object.freeze(descriptor);
}

export function resolveDependencyGraph(
  requests: readonly DependencyRequest[],
  descriptors: Iterable<PackageDescriptor>,
): DependencyGraph {
  return resolveGraph(requests, descriptors);
}

export function resolveDependencyGraphForTarget(
  requests: readonly DependencyRequest[],
  descriptors: Iterable<PackageDescriptor>,
  target: TargetSpec,
): DependencyGraph {
  return resolveGraph(requests, descriptors, (descriptor) =>
    ensureTargetCompatibility(descriptor, target),
  );
}

export function resolveForTargets(
  requests: readonly DependencyRequest[],
  descriptors: Iterable<PackageDescriptor>,
  targets: readonly TargetSpec[],
): ProjectResolution {
  validateTargets(targets);
  const sortedTargets = [...targets].sort((left, right) =>
    left.name.localeCompare(right.name),
  );
  const descriptorList = [...descriptors];
  const packages = new Map<string, { descriptor: PackageDescriptor; targets: Set<string> }>();

  const resolvedTargets = sortedTargets.map((target) => {
    const graph = resolveGraph(requests, descriptorList, (descriptor) =>
      ensureTargetCompatibility(descriptor, target),
    );
    for (const node of graph.nodes) {
      const key = formatPackageId(node.id);
      const entry = packages.get(key);
      if (entry === undefined) {
        packages.set(key, { descriptor: node.descriptor, targets: new Set([target.name]) });
      } else {
        entry.targets.add(target.name);
      }
    }
    return Object.freeze({ target: freezeTarget(target), graph });
  });

  const lockPackages = [...packages.entries()]
    .sort(([left], [right]) => left.localeCompare(right))
    .map(([key, entry]) => lockPackage(key, entry.descriptor, [...entry.targets].sort()));

  return Object.freeze({
    targets: Object.freeze(resolvedTargets),
    packages: Object.freeze(lockPackages),
  });
}

function resolveGraph(
  requests: readonly DependencyRequest[],
  descriptors: Iterable<PackageDescriptor>,
  check: (descriptor: PackageDescriptor) => void = () => {},
): DependencyGraph {
  const descriptorsById = new Map<string, PackageDescriptor>();
  const dependenciesById = new Map<string, readonly DependencyRequest[]>();

  for (const raw of descriptors) {
    const descriptor = createDescriptor({
      id: raw.id,
      dependencies: raw.dependencies,
      ...(raw.targets === undefined ? {} : { targets: raw.targets }),
      ...(raw.capabilities === undefined ? {} : { capabilities: raw.capabilities }),
      ...(raw.integrity === undefined ? {} : { integrity: raw.integrity }),
      ...(raw.resolved === undefined ? {} : { resolved: raw.resolved }),
      ...(raw.registry === undefined ? {} : { registry: raw.registry }),
      ...(raw.commit === undefined ? {} : { commit: raw.commit }),
      ...(raw.lifecycleScripts === undefined
        ? {}
        : { lifecycleScripts: raw.lifecycleScripts }),
      ...(raw.metadata === undefined ? {} : { metadata: raw.metadata }),
    });
    const key = formatPackageId(descriptor.id);
    if (descriptorsById.has(key)) {
      throw new NopmError("NP0002", `Duplicate package descriptor '${key}'.`, key);
    }
    descriptorsById.set(key, descriptor);

    const dependencies = Object.entries(descriptor.dependencies).map(([alias, spec]) =>
      parseDependencyRequest(alias, spec),
    );
    dependencies.sort(compareDependencies);
    dependenciesById.set(key, Object.freeze(dependencies));
  }

  const rootMap = new Map<string, DependencyRequest>();
  for (const request of requests) {
    const existing = rootMap.get(request.alias);
    if (existing !== undefined && formatPackageId(existing.id) !== formatPackageId(request.id)) {
      throw new NopmError(
        "NP0005",
        `Dependency alias '${request.alias}' resolves to both '${formatPackageId(existing.id)}' and '${formatPackageId(request.id)}'.`,
        request.alias,
      );
    }
    rootMap.set(request.alias, request);
  }

  const sortedRoots = [...rootMap.values()].sort(
    (left, right) =>
      formatPackageId(left.id).localeCompare(formatPackageId(right.id)) ||
      left.alias.localeCompare(right.alias),
  );

  const visiting = new Set<string>();
  const stack: string[] = [];
  const cache = new Map<string, DependencyGraphNode>();

  const visit = (id: string): DependencyGraphNode => {
    const cached = cache.get(id);
    if (cached !== undefined) {
      return cached;
    }
    if (visiting.has(id)) {
      throw new NopmError(
        "NP0004",
        `Dependency cycle: ${[...stack, id].join(" -> ")}.`,
        id,
      );
    }

    const descriptor = descriptorsById.get(id);
    if (descriptor === undefined) {
      throw new NopmError("NP0003", `No descriptor was supplied for '${id}'.`, id);
    }
    check(descriptor);

    visiting.add(id);
    stack.push(id);
    const children = (dependenciesById.get(id) ?? []).map((dependency) =>
      visit(formatPackageId(dependency.id)),
    );
    stack.pop();
    visiting.delete(id);

    const node = Object.freeze({
      id: descriptor.id,
      descriptor,
      dependencies: Object.freeze(children),
    });
    cache.set(id, node);
    return node;
  };

  const roots = sortedRoots.map((request) => visit(formatPackageId(request.id)));
  const nodes = [...cache.values()].sort((left, right) =>
    formatPackageId(left.id).localeCompare(formatPackageId(right.id)),
  );

  return Object.freeze({
    roots: Object.freeze(roots),
    nodes: Object.freeze(nodes),
  });
}

function validateTargets(targets: readonly TargetSpec[]): void {
  if (targets.length === 0) {
    throw new NopmError("NP0006", "At least one compiler target is required.", "targets");
  }

  const names = new Set<string>();
  for (const target of targets) {
    if (target.name.trim().length === 0 || target.name.trim() !== target.name) {
      throw new NopmError("NP0006", "Target names must be non-empty and trimmed.", target.name);
    }
    if (target.backend.trim().length === 0) {
      throw new NopmError(
        "NP0006",
        `Target '${target.name}' requires a non-empty backend.`,
        target.name,
      );
    }
    if (!(target.mode in MODE_ORDER)) {
      throw new NopmError(
        "NP0007",
        `Target '${target.name}' has unknown mode '${target.mode}'.`,
        target.mode,
      );
    }
    if (names.has(target.name)) {
      throw new NopmError("NP0006", `Duplicate target '${target.name}'.`, target.name);
    }
    names.add(target.name);
  }
}

function freezeTarget(target: TargetSpec): TargetSpec {
  return Object.freeze({
    name: target.name,
    backend: target.backend,
    mode: target.mode,
    ...(target.triple === undefined ? {} : { triple: target.triple }),
  });
}

function ensureTargetCompatibility(descriptor: PackageDescriptor, target: TargetSpec): void {
  const declared = descriptor.targets;
  if (declared !== undefined && declared.length > 0) {
    const candidates = new Set(
      [target.name, target.backend, target.triple]
        .filter((value): value is string => value !== undefined)
        .map((value) => value.toLowerCase()),
    );
    if (!declared.some((value) => candidates.has(value.toLowerCase()))) {
      throw new NopmError(
        "NP0006",
        `Package '${formatPackageId(descriptor.id)}' does not support target '${target.name}'.`,
        formatPackageId(descriptor.id),
      );
    }
  }

  const required = requiredMode(descriptor);
  if (required !== null && !modeAllows(target.mode, required)) {
    throw new NopmError(
      "NP0007",
      `Package '${formatPackageId(descriptor.id)}' requires '${required}' mode, but target '${target.name}' uses '${target.mode}'.`,
      formatPackageId(descriptor.id),
    );
  }
}

function requiredMode(descriptor: PackageDescriptor): Mode | null {
  let required: Mode | null = null;

  for (const capability of descriptor.capabilities ?? []) {
    const mode = CAPABILITY_MODES[capability];
    if (mode === undefined) {
      continue;
    }
    if (mode === "web" && required === "system") {
      throw incompatibleCapabilities(descriptor);
    }
    if (mode === "system" && required === "web") {
      throw incompatibleCapabilities(descriptor);
    }
    if (required === null || mode === "system" || (mode === "web" && required !== "system")) {
      required = mode;
    }
  }

  return required;
}

function incompatibleCapabilities(descriptor: PackageDescriptor): NopmError {
  return new NopmError(
    "NP0007",
    `Package '${formatPackageId(descriptor.id)}' requires both web and system capabilities, which no single mode allows.`,
    formatPackageId(descriptor.id),
  );
}

function modeAllows(selected: Mode, required: Mode): boolean {
  return MODE_ORDER[selected].includes(required);
}

function lockPackage(
  id: string,
  descriptor: PackageDescriptor,
  targets: readonly string[],
): LockfilePackage {
  const dependencies = Object.entries(descriptor.dependencies)
    .map(([alias, spec]) => formatPackageId(parseDependencyRequest(alias, spec).id))
    .sort();
  const capabilities = [...new Set(descriptor.capabilities ?? [])].sort();

  return Object.freeze({
    id,
    name: descriptor.id.name,
    source: descriptor.id.source,
    version: descriptor.id.version,
    resolved: descriptor.resolved ?? null,
    registry: descriptor.registry ?? null,
    integrity: descriptor.integrity ?? null,
    targets: Object.freeze([...targets]),
    capabilities: Object.freeze(capabilities),
    lifecycleScripts: descriptor.lifecycleScripts ?? false,
    dependencies: Object.freeze(dependencies),
    commit:
      descriptor.commit ??
      (descriptor.id.source === "git" ? descriptor.id.reference : null),
  });
}

export const LOCKFILE_NAME = "core.lock.json";

export function hashManifestSource(source: string): string {
  const normalized = source.replace(/^\uFEFF/, "").replace(/\r\n/g, "\n");
  return `sha256:${createHash("sha256").update(normalized, "utf8").digest("hex")}`;
}

export function isLockfileCurrent(lockfile: Lockfile, manifestSource: string): boolean {
  return lockfile.manifestHash === hashManifestSource(manifestSource);
}

export function assertLockfileCurrent(
  lockfile: Lockfile,
  manifestSource: string,
): Lockfile {
  const expected = hashManifestSource(manifestSource);
  if (lockfile.manifestHash !== expected) {
    throw new NopmError(
      "NP0009",
      `Lockfile is stale. Expected ${expected}, found ${lockfile.manifestHash}.`,
      LOCKFILE_NAME,
    );
  }
  return lockfile;
}

export function readLockfileFile(path: string): Lockfile {
  let source: string;
  try {
    source = readFileSync(path, "utf8");
  } catch (error) {
    if (typeof error === "object" && error !== null && "code" in error && error.code === "ENOENT") {
      throw new NopmError("NP0009", `Lockfile '${path}' does not exist.`, path, {
        cause: error,
      });
    }
    throw new NopmError("NP0008", `Unable to read lockfile '${path}'.`, path, {
      cause: error,
    });
  }
  return parseLockfile(source);
}

export function writeLockfileFile(path: string, lockfile: Lockfile): void {
  const source = formatLockfile(lockfile);
  const temporary = `${path}.tmp-${process.pid}-${Date.now()}`;
  try {
    writeFileSync(temporary, source, "utf8");
    renameSync(temporary, path);
  } catch (error) {
    try {
      rmSync(temporary, { force: true });
    } catch {

    }
    throw new NopmError("NP0010", `Unable to write lockfile '${path}'.`, path, {
      cause: error,
    });
  }
}

export function createLockfile(input: LockfileInput): Lockfile {
  if (input.generatedBy.trim().length === 0) {
    throw new NopmError("NP0008", "Lockfile generatedBy must not be empty.");
  }
  if (input.manifestHash.trim().length === 0) {
    throw new NopmError("NP0008", "Lockfile manifestHash must not be empty.");
  }
  if (input.compiler.version.trim().length === 0 || input.compiler.apiVersion.trim().length === 0) {
    throw new NopmError("NP0008", "Lockfile compiler version and apiVersion must not be empty.");
  }

  const targetMap: Record<string, LockfileTarget> = {};
  for (const resolved of [...input.resolution.targets].sort((left, right) =>
    left.target.name.localeCompare(right.target.name),
  )) {
    if (targetMap[resolved.target.name] !== undefined) {
      throw new NopmError(
        "NP0006",
        `Duplicate target '${resolved.target.name}' in resolution.`,
        resolved.target.name,
      );
    }
    targetMap[resolved.target.name] = Object.freeze({
      backend: resolved.target.backend,
      mode: resolved.target.mode,
      triple: resolved.target.triple ?? null,
    });
  }

  const packages = [...input.resolution.packages]
    .sort((left, right) => left.id.localeCompare(right.id))
    .map((entry) => {
      if (entry.source !== "path" && (entry.integrity === null || entry.integrity.length === 0)) {
        throw new NopmError(
          "NP0008",
          `Remote package '${entry.id}' requires an integrity hash.`,
          entry.id,
        );
      }
      if (entry.source === "git" && (entry.commit === null || entry.commit.length === 0)) {
        throw new NopmError(
          "NP0008",
          `Git package '${entry.id}' requires a pinned commit.`,
          entry.id,
        );
      }
      return Object.freeze({
        id: entry.id,
        name: entry.name,
        source: entry.source,
        version: entry.version,
        resolved: entry.resolved,
        registry: entry.registry,
        integrity: entry.integrity,
        targets: Object.freeze([...entry.targets]),
        capabilities: Object.freeze([...entry.capabilities]),
        lifecycleScripts: entry.lifecycleScripts,
        dependencies: Object.freeze([...entry.dependencies]),
        commit: entry.commit,
      });
    });

  return Object.freeze({
    lockfileVersion: 1,
    generatedBy: input.generatedBy,
    manifestHash: input.manifestHash,
    compiler: Object.freeze({
      version: input.compiler.version,
      apiVersion: input.compiler.apiVersion,
    }),
    semantics: Object.freeze(input.semantics ?? DEFAULT_SEMANTICS),
    toolchain: input.toolchain === undefined ? null : input.toolchain,
    targets: Object.freeze(targetMap),
    packages: Object.freeze(packages),
    ffi: Object.freeze({ ...(input.ffi ?? {}) }),
  });
}

export function formatLockfile(lockfile: Lockfile): string {
  const targets: Record<string, unknown> = {};
  for (const name of Object.keys(lockfile.targets).sort()) {
    const target = lockfile.targets[name];
    if (target === undefined) {
      continue;
    }
    targets[name] = {
      backend: target.backend,
      ...(target.triple === null ? {} : { triple: target.triple }),
      mode: target.mode,
    };
  }

  const packages = lockfile.packages.map((entry) => ({
    id: entry.id,
    name: entry.name,
    source: entry.source,
    version: entry.version,
    resolved: entry.resolved,
    registry: entry.registry,
    integrity: entry.integrity,
    targets: entry.targets,
    capabilities: entry.capabilities,
    lifecycleScripts: entry.lifecycleScripts,
    dependencies: entry.dependencies,
    ...(entry.commit === null ? {} : { commit: entry.commit }),
  }));

  const document = {
    lockfileVersion: lockfile.lockfileVersion,
    generatedBy: lockfile.generatedBy,
    manifestHash: lockfile.manifestHash,
    compiler: {
      version: lockfile.compiler.version,
      apiVersion: lockfile.compiler.apiVersion,
    },
    semantics: {
      integers: lockfile.semantics.integers,
      strings: lockfile.semantics.strings,
    },
    targets,
    packages,
    ffi: sortJson(lockfile.ffi),
    toolchain: lockfile.toolchain === null ? null : {
      rung: lockfile.toolchain.rung,
      id: lockfile.toolchain.id,
      family: lockfile.toolchain.family,
      path: lockfile.toolchain.path,
      version: lockfile.toolchain.version,
    },
  };

  return `${JSON.stringify(document, null, 2)}\n`;
}

export function parseLockfile(source: string): Lockfile {
  let value: unknown;
  try {
    value = JSON.parse(source);
  } catch (error) {
    throw new NopmError("NP0008", "Invalid lockfile JSON.", undefined, { cause: error });
  }
  return validateLockfile(value);
}

export function validateLockfile(value: unknown): Lockfile {
  if (!isRecord(value) || value["lockfileVersion"] !== 1) {
    throw lockfileError("Lockfile must be an object with lockfileVersion 1.");
  }

  const generatedBy = requireString(value, "generatedBy", "lockfile");
  const manifestHash = requireString(value, "manifestHash", "lockfile");
  const compiler = requireRecord(value, "compiler", "lockfile");
  const semantics = requireRecord(value, "semantics", "lockfile");
  const targetsRaw = requireRecord(value, "targets", "lockfile");
  const packagesRaw = requireArray(value, "packages", "lockfile");
  const ffi = requireRecord(value, "ffi", "lockfile");
  const toolchainRaw = value["toolchain"];
  let toolchain: LockfileToolchain | null = null;
  if (toolchainRaw !== null && toolchainRaw !== undefined) {
    if (!isRecord(toolchainRaw)) {
      throw lockfileError("Lockfile 'toolchain' must be an object or null.");
    }
    const rung = requireString(toolchainRaw, "rung", "toolchain");
    if (rung !== "env" && rung !== "cache" && rung !== "path") {
      throw new NopmError("NP0008", `Toolchain rung '${rung}' is invalid.`, "toolchain");
    }
    const id = requireString(toolchainRaw, "id", "toolchain");
    const family = requireString(toolchainRaw, "family", "toolchain");
    const path = requireString(toolchainRaw, "path", "toolchain");
    const version = optionalNullableString(toolchainRaw, "version", "toolchain");
    toolchain = Object.freeze({ rung: rung as "env" | "cache" | "path", id, family, path, version });
  }

  const targets: Record<string, LockfileTarget> = {};
  for (const name of Object.keys(targetsRaw).sort()) {
    const raw = targetsRaw[name];
    if (!isRecord(raw)) {
      throw lockfileError(`Target '${name}' must be an object.`);
    }
    const backend = requireString(raw, "backend", `target '${name}'`);
    const mode = requireString(raw, "mode", `target '${name}'`);
    if (!(mode in MODE_ORDER)) {
      throw new NopmError("NP0008", `Target '${name}' has invalid mode '${mode}'.`, name);
    }
    const triple = optionalNullableString(raw, "triple", `target '${name}'`);
    targets[name] = Object.freeze({ backend, mode: mode as Mode, triple });
  }

  const packages = packagesRaw.map((entry, index) => validateLockPackage(entry, index));
  const seen = new Set<string>();
  let previous = "";
  for (const entry of packages) {
    if (seen.has(entry.id)) {
      throw new NopmError("NP0008", `Duplicate lockfile package '${entry.id}'.`, entry.id);
    }
    if (entry.id.localeCompare(previous) < 0) {
      throw new NopmError(
        "NP0008",
        "Lockfile packages must be sorted by canonical package ID.",
        entry.id,
      );
    }
    seen.add(entry.id);
    previous = entry.id;
  }

  return Object.freeze({
    lockfileVersion: 1,
    generatedBy,
    manifestHash,
    compiler: Object.freeze({
      version: requireString(compiler, "version", "lockfile compiler"),
      apiVersion: requireString(compiler, "apiVersion", "lockfile compiler"),
    }),
    semantics: Object.freeze({
      integers: requireString(semantics, "integers", "lockfile semantics"),
      strings: requireString(semantics, "strings", "lockfile semantics"),
    }),
    targets: Object.freeze(targets),
    packages: Object.freeze(packages),
    ffi: Object.freeze({ ...ffi }),
    toolchain,
  });
}

function validateLockPackage(value: unknown, index: number): LockfilePackage {
  const label = `package[${index}]`;
  if (!isRecord(value)) {
    throw lockfileError(`${label} must be an object.`);
  }

  const rawId = requireString(value, "id", label);
  let id: PackageId;
  try {
    id = parsePackageId(rawId);
  } catch (error) {
    throw new NopmError("NP0008", `${label} has an invalid package ID '${rawId}'.`, rawId, {
      cause: error,
    });
  }
  if (formatPackageId(id) !== rawId) {
    throw new NopmError("NP0008", `${label} package ID is not canonical.`, rawId);
  }

  const source = requireString(value, "source", label);
  if (source !== id.source) {
    throw new NopmError("NP0008", `${label} source does not match its package ID.`, rawId);
  }

  const name = requireString(value, "name", label);
  if (name !== id.name) {
    throw new NopmError("NP0008", `${label} name does not match its package ID.`, rawId);
  }
  const version = nullableString(value, "version", label);
  const resolved = nullableString(value, "resolved", label);
  const registry = nullableString(value, "registry", label);
  const integrity = nullableString(value, "integrity", label);
  const targets = stringArray(value, "targets", label);
  const capabilities = stringArray(value, "capabilities", label);
  const lifecycleScripts = requireBoolean(value, "lifecycleScripts", label);
  const dependencies = stringArray(value, "dependencies", label);
  const commit = optionalNullableString(value, "commit", label);

  if (version !== id.version) {
    throw new NopmError("NP0008", `${label} version does not match its package ID.`, rawId);
  }
  if (id.source !== "path" && (integrity === null || integrity.length === 0)) {
    throw new NopmError("NP0008", `${label} remote package requires integrity.`, rawId);
  }
  if (id.source === "git" && (commit === null || commit.length === 0)) {
    throw new NopmError("NP0008", `${label} git package requires a pinned commit.`, rawId);
  }

  const canonicalDependencies = [...new Set(dependencies)].sort();
  if (
    canonicalDependencies.length !== dependencies.length ||
    canonicalDependencies.some((dependency, position) => dependency !== dependencies[position])
  ) {
    throw new NopmError(
      "NP0008",
      `${label} dependencies must be unique and sorted by canonical ID.`,
      rawId,
    );
  }

  for (const dependency of dependencies) {
    try {
      const dependencyId = parsePackageId(dependency);
      if (formatPackageId(dependencyId) !== dependency) {
        throw new NopmError("NP0008", `${label} dependency ID is not canonical.`, dependency);
      }
    } catch (error) {
      if (error instanceof NopmError && error.code === "NP0008") {
        throw error;
      }
      throw new NopmError("NP0008", `${label} has an invalid dependency ID.`, dependency, {
        cause: error,
      });
    }
  }

  return Object.freeze({
    id: rawId,
    name,
    source: id.source,
    version,
    resolved,
    registry,
    integrity,
    targets: Object.freeze([...targets]),
    capabilities: Object.freeze([...capabilities]),
    lifecycleScripts,
    dependencies: Object.freeze([...dependencies]),
    commit,
  });
}

function sortJson(value: unknown): unknown {
  if (Array.isArray(value)) {
    return value.map(sortJson);
  }
  if (isRecord(value)) {
    const sorted: Record<string, unknown> = {};
    for (const key of Object.keys(value).sort()) {
      sorted[key] = sortJson(value[key]);
    }
    return sorted;
  }
  return value;
}

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}

function requireRecord(
  value: Record<string, unknown>,
  key: string,
  label: string,
): Record<string, unknown> {
  const entry = value[key];
  if (!isRecord(entry)) {
    throw lockfileError(`${label}.${key} must be an object.`);
  }
  return entry;
}

function requireArray(
  value: Record<string, unknown>,
  key: string,
  label: string,
): unknown[] {
  const entry = value[key];
  if (!Array.isArray(entry)) {
    throw lockfileError(`${label}.${key} must be an array.`);
  }
  return entry;
}

function requireString(
  value: Record<string, unknown>,
  key: string,
  label: string,
): string {
  const entry = value[key];
  if (typeof entry !== "string" || entry.length === 0) {
    throw lockfileError(`${label}.${key} must be a non-empty string.`);
  }
  return entry;
}

function nullableString(
  value: Record<string, unknown>,
  key: string,
  label: string,
): string | null {
  const entry = value[key];
  if (entry === null) {
    return null;
  }
  if (typeof entry !== "string") {
    throw lockfileError(`${label}.${key} must be a string or null.`);
  }
  return entry;
}

function optionalNullableString(
  value: Record<string, unknown>,
  key: string,
  label: string,
): string | null {
  if (!(key in value)) {
    return null;
  }
  return nullableString(value, key, label);
}

function requireBoolean(
  value: Record<string, unknown>,
  key: string,
  label: string,
): boolean {
  const entry = value[key];
  if (typeof entry !== "boolean") {
    throw lockfileError(`${label}.${key} must be a boolean.`);
  }
  return entry;
}

function stringArray(
  value: Record<string, unknown>,
  key: string,
  label: string,
): string[] {
  const entry = value[key];
  if (!Array.isArray(entry) || entry.some((item) => typeof item !== "string")) {
    throw lockfileError(`${label}.${key} must be an array of strings.`);
  }
  return entry as string[];
}

function lockfileError(message: string): NopmError {
  return new NopmError("NP0008", message);
}

function parseNova(raw: string, rest: string): PackageId {
  const { name, version } = splitVersion(rest, raw);
  validateScopedOrPlainName(name, raw, "nova");
  if (version === null) {
    throw invalidId(raw, "Nova package IDs require an exact version, such as nova:name@1.2.3.");
  }
  if (!SEMVER.test(version)) {
    throw invalidId(raw, `Nova package version '${version}' is not an exact semantic version.`);
  }
  return makeId("nova", name, version, null, null, raw);
}

function parseNpm(raw: string, rest: string): PackageId {
  const { name, version } = splitVersion(rest, raw);
  validateScopedOrPlainName(name, raw, "npm");
  if (version !== null) {
    validateVersionText(version, raw);
  }
  return makeId("npm", name, version, null, null, raw);
}

function parseCargo(raw: string, rest: string): PackageId {
  const { name, version } = splitVersion(rest, raw);
  if (!CARGO_NAME.test(name)) {
    throw invalidId(raw, `Invalid Cargo package name '${name}'.`);
  }
  if (version !== null) {
    validateVersionText(version, raw);
  }
  return makeId("cargo", name, version, null, null, raw);
}

function parseEcosystem(
  raw: string,
  rest: string,
  source: "pypi" | "nuget",
): PackageId {
  const { name, version } = splitVersion(rest, raw);
  if (!ECOSYSTEM_NAME.test(name)) {
    throw invalidId(raw, `Invalid ${source} package name '${name}'.`);
  }
  if (version !== null) {
    validateVersionText(version, raw);
  }
  return makeId(source, name, version, null, null, raw);
}

function parseOci(raw: string, rest: string): PackageId {
  const separator = rest.lastIndexOf("@");
  if (separator <= 0 || separator === rest.length - 1) {
    throw invalidId(raw, "OCI package IDs require registry/repository@reference.");
  }
  const name = rest.slice(0, separator);
  const reference = rest.slice(separator + 1);
  const segments = name.split("/");
  if (segments.length < 2 || segments.some((segment) => segment.length === 0)) {
    throw invalidId(raw, "OCI package names require registry/repository.");
  }
  if (/\s/.test(name) || /\s/.test(reference) || reference.includes("@")) {
    throw invalidId(raw, "OCI names and references must not contain whitespace or '@'.");
  }
  return makeId("oci", name, null, reference, null, raw);
}

function parseGit(raw: string): PackageId {
  const rest = raw.slice("git+".length);
  const hash = rest.lastIndexOf("#");
  if (hash <= 0 || hash === rest.length - 1) {
    throw invalidId(raw, "Git package IDs require a URL and a pinned #reference.");
  }

  const url = rest.slice(0, hash);
  const reference = rest.slice(hash + 1);
  if (!/^(https?|ssh):\/\//i.test(url) || /\s/.test(url) || url.includes("#")) {
    throw invalidId(raw, "Git package URLs must use http, https, or ssh and contain no whitespace.");
  }
  if (reference.length === 0 || /\s/.test(reference) || reference.includes("#")) {
    throw invalidId(raw, "Git references must be non-empty and contain no whitespace or '#'.");
  }

  return makeId("git", url, null, reference, null, raw);
}

function parsePath(raw: string, rest: string): PackageId {
  if (rest.length === 0 || rest.trim() !== rest || rest.includes("\0")) {
    throw invalidId(raw, "Path package IDs require a non-empty path without surrounding whitespace.");
  }

  const segments = rest.split(/[\\/]+/).filter((segment) => segment.length > 0);
  const name = segments.at(-1) ?? rest;
  return makeId("path", name, null, null, rest, raw);
}

function splitVersion(rest: string, raw: string): { name: string; version: string | null } {
  const scoped = rest.startsWith("@");
  const at = rest.indexOf("@", scoped ? 1 : 0);
  if (scoped && at <= 1) {
    throw invalidId(raw, "Scoped package names must use @scope/name.");
  }
  if (at < 0) {
    return { name: rest, version: null };
  }
  const name = rest.slice(0, at);
  const version = rest.slice(at + 1);
  if (version.length === 0) {
    throw invalidId(raw, "Package versions after '@' must not be empty.");
  }
  return { name, version };
}

function validateScopedOrPlainName(name: string, raw: string, source: PackageSource): void {
  if (name.startsWith("@")) {
    if (!SCOPED_NAME.test(name)) {
      throw invalidId(raw, `Invalid ${source} scoped package name '${name}'.`);
    }
  } else if (!NPM_NAME.test(name)) {
    throw invalidId(raw, `Invalid ${source} package name '${name}'.`);
  }
}

function validateVersionText(version: string, raw: string): void {
  if (version.length === 0 || version.trim() !== version || version.includes("@")) {
    throw invalidId(raw, `Invalid package version or range '${version}'.`);
  }
}

function makeId(
  source: PackageSource,
  name: string,
  version: string | null,
  reference: string | null,
  path: string | null,
  raw: string,
): PackageId {
  return Object.freeze({ source, name, version, reference, path, raw });
}

function invalidId(input: string, message: string): NopmError {
  return new NopmError("NP0001", message, input);
}

function compareDependencies(left: DependencyRequest, right: DependencyRequest): number {
  return (
    formatPackageId(left.id).localeCompare(formatPackageId(right.id)) ||
    left.alias.localeCompare(right.alias)
  );
}
