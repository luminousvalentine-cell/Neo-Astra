#!/usr/bin/env node

import { spawnSync } from "node:child_process";
import {
  existsSync,
  readFileSync,
  readdirSync,
  renameSync,
  rmSync,
  writeFileSync,
} from "node:fs";
import { join, resolve } from "node:path";

import { ManifestError } from "@neo-astra/manifest";
import { addDependency, findCoreManifest, loadBuildManifest, removeDependency } from "@neo-astra/manifest";
import type { LockfileToolchain } from "./index.js";
import { parseToolchainPolicy, resolveToolchainAsync, ToolchainError, type ToolchainProbe } from "@neo-astra/toolchain";

import {
  LOCKFILE_NAME,
  NopmError,
  assertLockfileCurrent,
  createLockfile,
  formatLockfile,
  formatPackageId,
  parseDependencyRequest,
  parsePackageId,
  readLockfileFile,
  resolveForTargets,
  type DependencyRequest,
  type PackageDescriptor,
} from "./index.js";
import { defaultCommandRunner } from "./command.js";
import { planSysInstall, executeSysInstall, detectInstalled, detectManagers, assertKnownSysTool } from "./sys-adapter.js";
import { resolvePathPackage } from "./path-adapter.js";
import { resolveNpmPackage, type AdapterResolution } from "./npm-adapter.js";
import { resolveGitPackage } from "./git-adapter.js";
import { resolveCargoPackage } from "./cargo-adapter.js";
import { resolvePyPiPackage } from "./pypi-adapter.js";
import { resolveNuGetPackage } from "./nuget-adapter.js";
import { resolveOciPackage } from "./oci-adapter.js";
import { resolveRegistryPackage } from "./registry-client.js";
import { installLockedPackages } from "./store.js";
import { readProject, type ProjectManifest } from "./project.js";

const VERSION = "1.0.0";
const GENERATED_BY = `nopm/${VERSION}`;
const COMPILER = { version: VERSION, apiVersion: "1.0" } as const;

const HELP = `NoPM universal package manager

Usage:
  nopm <command> [options]

Commands:
  add <id>[@version] [dir] [--json] [--dry-run]
                              Add a dependency; use sys:<tool> for git/cmake/clang
  remove <name> [dir] [--json] [--dry-run]
                              Remove a dependency and update the lockfile
  lock [dir] [--json]         Resolve dependencies and write core.lock.json
  tree [dir] [--json]        Print the locked dependency package list
  install [dir] [--locked|--frozen] [--json]
                             Verify and install locked packages
  search <query> [--registry <url>] [--json]
                             Search a registry gateway
  audit <id> [--registry <url>] [--json]
                             Read registry audit metadata for a package
  why <name>                 Show packages that depend on a package
  update [dir] [--json]      Re-resolve the lockfile
  version                    Print the NoPM version
  help                       Show this help

Current adapters:
  path, git, npm, cargo, pypi, nuget, oci

Staged adapters:
  none

Exit codes:
  0  success
  1  package, lockfile, or installation failure
  2  usage error
`;

interface CliOptions {
  readonly command: string;
  readonly directory: string;
  readonly target: string | null;
  readonly registry: string | null;
  readonly json: boolean;
  readonly locked: boolean;
  readonly frozen: boolean;
  readonly dryRun: boolean;
}

interface Diagnostic {
  readonly code: string;
  readonly message: string;
  readonly path: string | null;
}

export async function runNopm(argv: readonly string[]): Promise<number> {
  try {
    const options = parseArgs(argv);
    switch (options.command) {
      case "help":
        process.stdout.write(HELP);
        return 0;
      case "version":
        process.stdout.write(`nopm ${VERSION}\n`);
        return 0;
      case "add":
        return await runAdd(options);
      case "remove":
        return await runRemove(options);
      case "lock":
        return await runLock(options);
      case "update":
        return await runLock({ ...options, command: "update" });
      case "tree":
        return runTree(options);
      case "install":
        return await runInstall(options);
      case "search":
        return await runSearch(options);
      case "audit":
        return await runAudit(options);
      case "why":
        return runWhy(options);
      default:
        throw usage(`Unknown command '${options.command}'.`);
    }
  } catch (error) {
    return emitError(error, argv.includes("--json"));
  }
}

function parseArgs(args: readonly string[]): CliOptions {
  const json = args.includes("--json");
  const locked = args.includes("--locked");
  const frozen = args.includes("--frozen");
  const dryRun = args.includes("--dry-run");
  const positional: string[] = [];
  let registry: string | null = null;

  for (let index = 0; index < args.length; index += 1) {
    const argument = args[index];
    if (argument === undefined) {
      continue;
    }
    if (argument === "--json" || argument === "--locked" || argument === "--frozen" || argument === "--dry-run") {
      continue;
    }
    if (argument === "--registry") {
      const value = args[index + 1];
      if (value === undefined || value.startsWith("-")) {
        throw usage("The --registry option requires a base URL.");
      }
      registry = value;
      index += 1;
      continue;
    }
    if (argument === "-h" || argument === "--help") {
      return baseOptions("help", json, locked, frozen, dryRun, registry);
    }
    if (argument === "-v" || argument === "--version") {
      return baseOptions("version", json, locked, frozen, dryRun, registry);
    }
    if (argument.startsWith("-")) {
      throw usage(`Unknown option '${argument}'.`);
    }
    positional.push(argument);
  }

  const [command, ...rest] = positional;
  if (command === undefined) {
    return baseOptions("help", json, locked, frozen, dryRun, registry);
  }
  const isSys = rest[0]?.startsWith("sys:") === true;
  const maxPositionals = (command === "add" || command === "remove") && !isSys ? 2 : 1;
  if (rest.length > maxPositionals) {
    throw usage(`nopm ${command} accepts at most ${maxPositionals} argument(s).`);
  }
  if (locked && frozen) {
    throw usage("Use only one of --locked or --frozen.");
  }

  const addRemoveCommands = new Set(["add", "remove"]);
  if (addRemoveCommands.has(command)) {
    return {
      command,
      directory: resolve(process.cwd(), rest[1] ?? "."),
      target: rest[0] ?? null,
      registry,
      json,
      locked,
      frozen,
      dryRun,
    };
  }

  const targetCommands = new Set(["search", "audit", "why"]);
  if (targetCommands.has(command)) {
    return {
      command,
      directory: process.cwd(),
      target: rest[0] ?? null,
      registry,
      json,
      locked,
      frozen,
      dryRun,
    };
  }

  return {
    command,
    directory: resolve(process.cwd(), rest[0] ?? "."),
    target: null,
    registry,
    json,
    locked,
    frozen,
    dryRun,
  };
}

function baseOptions(
  command: string,
  json: boolean,
  locked: boolean,
  frozen: boolean,
  dryRun: boolean,
  registry: string | null,
): CliOptions {
  return {
    command,
    directory: process.cwd(),
    target: null,
    registry,
    json,
    locked,
    frozen,
    dryRun,
  };
}

function runLock(options: CliOptions): Promise<number> {
  return runLockAsync(options);
}

async function runLockAsync(options: CliOptions): Promise<number> {
  const project = readProject(options.directory);
  const requests = project.dependencies.map((dependency) => dependency.request);
  const resolved = await collectDescriptors(project, requests);
  const resolution = resolveForTargets(
    resolved.requests,
    resolved.descriptors,
    project.targets,
  );
  const toolchain = await resolveProjectToolchain(project);
  const lockfile = createLockfile({
    resolution,
    generatedBy: GENERATED_BY,
    manifestHash: project.manifestHash,
    compiler: COMPILER,
    ffi: project.ffi,
    toolchain,
  });
  const path = join(project.directory, LOCKFILE_NAME);
  writeAtomically(path, formatLockfile(lockfile));

  if (options.json) {
    process.stdout.write(
      `${JSON.stringify(
        {
          ok: true,
          command: options.command,
          lockfile: path,
          packages: lockfile.packages.length,
          targets: Object.keys(lockfile.targets),
        },
        null,
        2,
      )}\n`,
    );
  } else {
    process.stdout.write(
      `Locked ${lockfile.packages.length} package(s) for ${Object.keys(lockfile.targets).length} target(s).\n`,
    );
  }
  return 0;
}

async function runAdd(options: CliOptions): Promise<number> {
  const id = options.target;
  if (id === null) {
    throw usage("nopm add requires a package ID.");
  }

  if (id.startsWith("sys:")) {
    const tool = id.slice("sys:".length);

    assertKnownSysTool(tool);
    const context = {
      platform: process.platform,
      uid: typeof process.getuid === "function" ? process.getuid() : 0,
      managers: detectManagers(),
    };

    if (detectInstalled(tool, defaultCommandRunner)) {
      if (options.dryRun) {
        if (options.json) {
          process.stdout.write(JSON.stringify({ ok: true, dryRun: true, action: "sys-install", tool, manager: null, commands: [], installed: true }) + "\n");
        } else {
          process.stdout.write("already installed\n");
        }
        return 0;
      }
      if (options.json) {
        process.stdout.write(JSON.stringify({ ok: true, action: "sys-install", tool, manager: null, commands: [], installed: true }) + "\n");
      } else {
        process.stdout.write("already installed\n");
      }
      return 0;
    }

    const plan = planSysInstall(tool, context);
    if (options.dryRun) {
      if (options.json) {
        process.stdout.write(JSON.stringify({ ok: true, dryRun: true, action: "sys-install", tool, manager: plan.manager, commands: plan.commands, installed: false }) + "\n");
      } else {
        for (const cmd of plan.commands) process.stdout.write(cmd.join(" ") + "\n");
      }
      return 0;
    }

    const result = executeSysInstall(tool, context, defaultCommandRunner);
    if (options.json) {
      process.stdout.write(JSON.stringify({ ok: true, action: "sys-install", tool, manager: result.manager, commands: result.commands, installed: result.installed }) + "\n");
    } else {
      process.stdout.write(`Installed ${tool} via ${result.manager}.\n`);
    }
    return 0;
  }

  const parsedId = parsePackageId(id);
  const alias = (parsedId.name.split("@")[0] ?? parsedId.name).replace(/[^A-Za-z0-9_-]/g, "-").replace(/^-/, "_");
  const spec = id;

  const project = readProject(options.directory, { requireTargets: false });

  const existingSpec = project.dependencies.find((d) => d.alias === alias);
  if (existingSpec !== undefined) {
    throw new NopmError("NP0013", `Dependency '${alias}' already exists.`, alias);
  }

  if (options.dryRun) {
    if (options.json) {
      process.stdout.write(JSON.stringify({ ok: true, dryRun: true, action: "add", id: spec }) + "\n");
    } else {
      process.stdout.write(`Would add ${spec} to ${project.directory}\n`);
    }
    return 0;
  }

  const coreFile = findCoreManifest(project.directory);
  const originalManifest = readFileSync(coreFile.path, "utf8");
  const lockfilePath = join(project.directory, LOCKFILE_NAME);
  let originalLockfile: string | null = null;
  try {
    originalLockfile = readFileSync(lockfilePath, "utf8");
  } catch {
    originalLockfile = null;
  }
  let manifestWritten = false;
  let lockfileWritten = false;
  try {
    await addDependency(coreFile, alias, spec);
    manifestWritten = true;

    const newProject = readProject(options.directory, { requireTargets: false });
    const requests = newProject.dependencies.map((d) => d.request);
    const resolved = await collectDescriptors(newProject, requests);
    const resolution = resolveForTargets(resolved.requests, resolved.descriptors, newProject.targets);
    const toolchain = await resolveProjectToolchain(newProject);
    const lockfile = createLockfile({
      resolution,
      generatedBy: GENERATED_BY,
      manifestHash: newProject.manifestHash,
      compiler: COMPILER,
      ffi: newProject.ffi,
      toolchain,
    });
    writeAtomically(lockfilePath, formatLockfile(lockfile));
    lockfileWritten = true;

    await installLockedPackages(newProject.directory, lockfile, { generatedBy: GENERATED_BY });

    if (options.json) {
      process.stdout.write(JSON.stringify({ ok: true, action: "add", id: spec, packages: lockfile.packages.length }) + "\n");
    } else {
      process.stdout.write(`Added ${spec} (alias: ${alias}).\n`);
    }
    return 0;
  } catch (error) {
    if (manifestWritten) {
      writeFileSync(coreFile.path, originalManifest, "utf8");
    }
    if (lockfileWritten) {
      if (originalLockfile === null) {
        try { rmSync(lockfilePath, { force: true }); } catch {   }
      } else {
        writeFileSync(lockfilePath, originalLockfile, "utf8");
      }
    }
    throw error;
  }
}

async function runRemove(options: CliOptions): Promise<number> {
  const name = options.target;
  if (name === null) {
    throw usage("nopm remove requires a package name.");
  }

  if (name.startsWith("sys:")) {
    throw new NopmError("NP0016", "NoPM does not uninstall system tools.", name);
  }
  const project = readProject(options.directory, { requireTargets: false });

  const existing = project.dependencies.find((d) => d.request.alias === name);
  if (existing === undefined) {
    throw new NopmError("NP0014", `Dependency '${name}' is not declared.`, name);
  }

  if (options.dryRun) {
    if (options.json) {
      process.stdout.write(JSON.stringify({ ok: true, dryRun: true, action: "remove", name }) + "\n");
    } else {
      process.stdout.write(`Would remove ${name} from ${project.directory}\n`);
    }
    return 0;
  }

  const coreFile = findCoreManifest(project.directory);
  const originalManifest = readFileSync(coreFile.path, "utf8");
  const lockfilePath = join(project.directory, LOCKFILE_NAME);
  let originalLockfile: string | null = null;
  try {
    originalLockfile = readFileSync(lockfilePath, "utf8");
  } catch {
    originalLockfile = null;
  }
  let manifestWritten = false;
  let lockfileWritten = false;
  try {
    const removed = await removeDependency(coreFile, name);
    if (!removed) {
      throw new NopmError("NP0014", `Dependency '${name}' is not declared.`, name);
    }
    manifestWritten = true;

    const newProject = readProject(options.directory, { requireTargets: false });
    const requests = newProject.dependencies.map((d) => d.request);
    const resolved = await collectDescriptors(newProject, requests);
    const resolution = resolveForTargets(resolved.requests, resolved.descriptors, newProject.targets);
    const toolchain = await resolveProjectToolchain(newProject);
    const lockfile = createLockfile({
      resolution,
      generatedBy: GENERATED_BY,
      manifestHash: newProject.manifestHash,
      compiler: COMPILER,
      ffi: newProject.ffi,
      toolchain,
    });
    writeAtomically(lockfilePath, formatLockfile(lockfile));
    lockfileWritten = true;

    await installLockedPackages(newProject.directory, lockfile, { generatedBy: GENERATED_BY });

    if (options.json) {
      process.stdout.write(JSON.stringify({ ok: true, action: "remove", name }) + "\n");
    } else {
      process.stdout.write(`Removed ${name}.\n`);
    }
    return 0;
  } catch (error) {
    if (manifestWritten) {
      writeFileSync(coreFile.path, originalManifest, "utf8");
    }
    if (lockfileWritten) {
      if (originalLockfile === null) {
        try { rmSync(lockfilePath, { force: true }); } catch {   }
      } else {
        writeFileSync(lockfilePath, originalLockfile, "utf8");
      }
    }
    throw error;
  }
}

function runTree(options: CliOptions): number {
  const project = readProject(options.directory);
  const lockfile = currentLockfile(project, options);
  const packages = lockfile.packages;

  if (options.json) {
    process.stdout.write(
      `${JSON.stringify({ ok: true, command: "tree", packages }, null, 2)}\n`,
    );
  } else {
    for (const entry of packages) {
      const targets = entry.targets.length === 0 ? "universal" : entry.targets.join(", ");
      process.stdout.write(`${entry.id} [${targets}]\n`);
      for (const dependency of entry.dependencies) {
        process.stdout.write(`  -> ${dependency}\n`);
      }
    }
  }
  return 0;
}

function runInstall(options: CliOptions): Promise<number> {
  return runInstallAsync(options);
}

async function runInstallAsync(options: CliOptions): Promise<number> {
  const project = readProject(options.directory);
  const lockfile = currentLockfile(project, options);
  const store = await installLockedPackages(project.directory, lockfile, {
    generatedBy: GENERATED_BY,
  });

  if (options.json) {
    process.stdout.write(
      `${JSON.stringify(
        {
          ok: true,
          command: "install",
          packages: store.packages.length,
          store: join(project.directory, ".astra", "store", "index.json"),
        },
        null,
        2,
      )}\n`,
    );
  } else {
    process.stdout.write(`Installed ${store.packages.length} package(s).\n`);
  }
  return 0;
}

async function resolveProjectToolchain(project: ProjectManifest): Promise<LockfileToolchain | null> {
  const buildData = (loadBuildManifest(project.directory)?.data ?? {}) as Record<string, unknown>;
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
  try {
    const r = await resolveToolchainAsync({ projectRoot: project.directory, policy, env, probe, allowDownload: true });
    return { rung: r.rung, id: r.id, family: r.family, path: r.path, version: r.version };
  } catch (error) {

    if (policy.request === "auto" && error instanceof ToolchainError && error.code === "NS9102") {
      return null;
    }
    throw error;
  }
}
function currentLockfile(project: ProjectManifest, options: CliOptions) {
  const lockfile = readLockfileFile(join(project.directory, LOCKFILE_NAME));
  if (options.locked || options.frozen) {
    assertLockfileCurrent(lockfile, project.source);
  } else if (lockfile.manifestHash !== project.manifestHash) {
    throw new NopmError(
      "NP0009",
      "Lockfile is stale. Run `nopm lock` before installing.",
      LOCKFILE_NAME,
    );
  }
  return lockfile;
}

function collectDescriptors(
  project: ProjectManifest,
  requests: readonly DependencyRequest[],
): Promise<{ descriptors: PackageDescriptor[]; requests: DependencyRequest[] }> {
  return collectDescriptorsAsync(project, requests);
}

async function collectDescriptorsAsync(
  project: ProjectManifest,
  requests: readonly DependencyRequest[],
): Promise<{ descriptors: PackageDescriptor[]; requests: DependencyRequest[] }> {
  const descriptors = new Map<string, PackageDescriptor>();
  const exactRoots: DependencyRequest[] = [];
  const queue = requests.map((request) => ({ request, root: true }));

  while (queue.length > 0) {
    const item = queue.shift();
    if (item === undefined) {
      break;
    }
    const key = formatPackageId(item.request.id);
    if (descriptors.has(key)) {
      if (item.root) {
        exactRoots.push(item.request);
      }
      continue;
    }
    if (project.packageSources[item.request.id.source] === false) {
      throw new NopmError(
        "NP0011",
        `The '${item.request.id.source}' package source is disabled in the manifest.`,
        key,
      );
    }

    let resolved: AdapterResolution;
    switch (item.request.id.source) {
      case "path": {
        const descriptor = resolvePathPackage(project.directory, item.request.id);
        resolved = {
          descriptor,
          request: parseDependencyRequest(item.request.alias, formatPackageId(descriptor.id)),
        };
        break;
      }
      case "npm":
        resolved = resolveNpmPackage(item.request.id, item.request.alias);
        break;
      case "git":
        resolved = resolveGitPackage(
          item.request.id,
          item.request.alias,
          join(project.directory, ".astra", "cache"),
        );
        break;
      case "cargo":
        resolved = await resolveCargoPackage(item.request.id, item.request.alias);
        break;
      case "pypi":
        resolved = await resolvePyPiPackage(item.request.id, item.request.alias);
        break;
      case "nuget":
        resolved = await resolveNuGetPackage(item.request.id, item.request.alias);
        break;
      case "oci":
        resolved = await resolveOciPackage(item.request.id, item.request.alias);
        break;
      case "nova":
        if (project.registry === null) {
          throw new NopmError(
            "NP0011",
            `No [registry].default is configured; cannot resolve '${key}'.`,
            key,
          );
        }
        resolved = await resolveRegistryPackage(item.request.id, item.request.alias, {
          registry: project.registry,
        });
        break;
      default:
        throw new NopmError(
          "NP0011",
          `Unsupported package source '${item.request.id.source}'.`,
          key,
        );
    }

    const exactKey = formatPackageId(resolved.descriptor.id);
    if (!descriptors.has(exactKey)) {
      descriptors.set(exactKey, resolved.descriptor);
      for (const [alias, spec] of Object.entries(resolved.descriptor.dependencies)) {
        queue.push({ request: parseDependencyRequest(alias, spec), root: false });
      }
    }
    if (item.root) {
      exactRoots.push(resolved.request);
    }
  }

  return {
    descriptors: [...descriptors.values()].sort((left, right) =>
      formatPackageId(left.id).localeCompare(formatPackageId(right.id)),
    ),
    requests: exactRoots,
  };
}

function runWhy(options: CliOptions): number {
  const target = requiredTarget(options, "package name");
  const project = readProject(options.directory);
  const lockfile = currentLockfile(project, options);
  const matches = lockfile.packages.filter((entry) =>
    entry.dependencies.some((dependency) => dependencyMatches(dependency, target)),
  );

  if (options.json) {
    process.stdout.write(
      `${JSON.stringify(
        {
          ok: true,
          command: "why",
          target,
          packages: matches.map((entry) => entry.id),
        },
        null,
        2,
      )}\n`,
    );
  } else if (matches.length === 0) {
    process.stdout.write(`No locked package depends on ${target}.\n`);
  } else {
    for (const entry of matches) {
      process.stdout.write(`${entry.id} depends on ${target}\n`);
    }
  }
  return 0;
}

async function runSearch(options: CliOptions): Promise<number> {
  const query = requiredTarget(options, "search query");
  const registry = registryFor(options);
  const response = await fetch(`${registry}/v1/search?q=${encodeURIComponent(query)}`, {
    headers: { Accept: "application/json", "User-Agent": "neo-astra-nopm/0.1.0" },
  });
  if (!response.ok) {
    throw new NopmError("NP0011", `Registry search failed: HTTP ${response.status}.`, registry);
  }
  const value = (await response.json()) as unknown;
  if (options.json) {
    process.stdout.write(`${JSON.stringify({ ok: true, command: "search", result: value }, null, 2)}\n`);
    return 0;
  }
  if (!isRecord(value) || !Array.isArray(value["results"])) {
    throw new NopmError("NP0011", "Registry returned an invalid search response.", registry);
  }
  for (const result of value["results"]) {
    if (isRecord(result)) {
      process.stdout.write(`${String(result["name"])}@${String(result["version"])}\n`);
    }
  }
  return 0;
}

async function runAudit(options: CliOptions): Promise<number> {
  const target = requiredTarget(options, "package ID");
  const id = parsePackageId(target);
  if (id.version === null) {
    throw usage("nopm audit requires an exact package version.");
  }
  const registry = registryFor(options);
  const response = await fetch(
    `${registry}/v1/audit/${encodeURIComponent(id.name)}/${encodeURIComponent(id.version)}`,
    { headers: { Accept: "application/json", "User-Agent": "neo-astra-nopm/0.1.0" } },
  );
  if (!response.ok) {
    throw new NopmError("NP0011", `Registry audit failed: HTTP ${response.status}.`, registry);
  }
  const value = (await response.json()) as unknown;
  if (options.json) {
    process.stdout.write(`${JSON.stringify({ ok: true, command: "audit", result: value }, null, 2)}\n`);
  } else if (isRecord(value)) {
    process.stdout.write(
      `${id.name}@${id.version}: ${value["yanked"] === true ? "yanked" : "not yanked"}, ${Array.isArray(value["advisories"]) ? value["advisories"].length : 0} advisory(ies)\n`,
    );
  }
  return 0;
}

function registryFor(options: CliOptions): string {
  if (options.registry !== null) {
    return options.registry;
  }
  try {
  const project = readProject(options.directory);
    if (project.registry !== null) {
      return project.registry;
    }
  } catch {

  }
  throw usage("No registry is configured. Pass --registry <url> or set [registry].default.");
}

function requiredTarget(options: CliOptions, label: string): string {
  if (options.target === null) {
    throw usage(`nopm ${options.command} requires a ${label}.`);
  }
  return options.target;
}

function dependencyMatches(dependency: string, target: string): boolean {
  if (dependency === target) {
    return true;
  }
  try {
    return parsePackageId(dependency).name === target;
  } catch {
    return false;
  }
}

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}

function writeAtomically(path: string, source: string): void {
  const temporary = `${path}.tmp-${process.pid}-${Date.now()}`;
  try {
    writeFileSync(temporary, source, "utf8");
    renameSync(temporary, path);
  } catch (error) {
    try {
      rmSync(temporary, { force: true });
    } catch {

    }
    throw error;
  }
}

function usage(message: string): NopmError {
  return new NopmError("NP0012", message);
}

function emitError(error: unknown, json: boolean): number {
  const diagnostic = diagnosticFor(error);
  if (json) {
    process.stdout.write(
      `${JSON.stringify({ ok: false, diagnostics: [diagnostic] }, null, 2)}\n`,
    );
  } else {
    process.stderr.write(`${diagnostic.code}: ${diagnostic.message}\n`);
  }
  return error instanceof NopmError && error.code === "NP0012" ? 2 : 1;
}

function diagnosticFor(error: unknown): Diagnostic {
  if (error instanceof NopmError) {
    return { code: error.code, message: error.message, path: error.input ?? null };
  }
  if (error instanceof ToolchainError) {
    return { code: error.code, message: error.message, path: null };
  }
  if (error instanceof ManifestError) {
    return {
      code: error.code,
      message: error.message,
      path: error.path ?? null,
    };
  }
  return {
    code: "NP0010",
    message: error instanceof Error ? error.message : String(error),
    path: null,
  };
}

const entry = process.argv[1];
const isDirectEntry =
  entry !== undefined && /(?:^|[\\/])cli\.(?:js|ts)$/.test(entry);
if (isDirectEntry) {
  void runNopm(process.argv.slice(2)).then(
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
