import {
  NopmError,
  createDescriptor,
  formatPackageId,
  parseDependencyRequest,
  parsePackageId,
  type DependencyRequest,
  type PackageDescriptor,
  type PackageId,
} from "./index.js";
import { commandName, defaultCommandRunner, type CommandRunner } from "./command.js";

export interface AdapterResolution {
  readonly descriptor: PackageDescriptor;
  readonly request: DependencyRequest;
}

export function resolveNpmPackage(
  id: PackageId,
  alias: string,
  runner: CommandRunner = defaultCommandRunner,
): AdapterResolution {
  if (id.source !== "npm") {
    throw new NopmError("NP0011", "Expected an npm package ID.", formatPackageId(id));
  }

  const requested = id.version === null ? id.name : `${id.name}@${id.version}`;
  const versionJson = runNpm(
    runner,
    ["view", requested, "version", "--json"],
    `resolve npm package '${requested}'`,
  );
  const version = selectedVersion(versionJson, requested);
  const exactName = `${id.name}@${version}`;
  const metadataJson = runNpm(
    runner,
    ["view", exactName, "--json"],
    `read npm metadata for '${exactName}'`,
  );
  const metadata = selectMetadata(metadataJson, version, exactName);

  const name = stringField(metadata, "name") ?? id.name;
  if (name !== id.name) {
    throw new NopmError(
      "NP0011",
      `npm returned package '${name}' while resolving '${id.name}'.`,
      exactName,
    );
  }
  const returnedVersion = stringField(metadata, "version");
  if (returnedVersion !== version) {
    throw new NopmError(
      "NP0011",
      `npm returned version '${returnedVersion ?? "<missing>"}' while resolving '${exactName}'.`,
      exactName,
    );
  }

  const dependencies = metadata["dependencies"];
  const dependencyMap: Record<string, string> = {};
  if (isRecord(dependencies)) {
    for (const [dependency, spec] of Object.entries(dependencies)) {
      if (typeof spec === "string") {
        dependencyMap[dependency] = spec;
      }
    }
  }

  const dist = isRecord(metadata["dist"]) ? metadata["dist"] : {};
  const tarball = typeof dist["tarball"] === "string" ? dist["tarball"] : null;
  const integrity = typeof dist["integrity"] === "string" ? dist["integrity"] : null;
  if (tarball === null || integrity === null || integrity.length === 0) {
    throw new NopmError(
      "NP0011",
      `npm metadata for '${exactName}' is missing a tarball or integrity hash.`,
      exactName,
    );
  }
  let registry: string | null = null;
  try {
    registry = new URL(tarball).origin;
  } catch {
    registry = null;
  }

  const exactId = parsePackageId(`npm:${name}@${version}`);
  const descriptor = createDescriptor({
    id: exactId,
    dependencies: dependencyMap,
    integrity,
    resolved: tarball,
    ...(registry === null ? {} : { registry }),
    lifecycleScripts: false,
  });

  return Object.freeze({
    descriptor,
    request: parseDependencyRequest(alias, formatPackageId(exactId)),
  });
}

function runNpm(
  runner: CommandRunner,
  args: readonly string[],
  action: string,
): unknown {
  const result = runner.run(commandName("npm"), args);
  if (result.status !== 0) {
    const detail = result.stderr.trim();
    throw new NopmError(
      "NP0011",
      `Unable to ${action}${detail.length === 0 ? "." : `: ${detail}`}`,
    );
  }
  try {
    return JSON.parse(result.stdout) as unknown;
  } catch (error) {
    throw new NopmError("NP0011", `npm returned invalid JSON while trying to ${action}.`, undefined, {
      cause: error,
    });
  }
}

function selectedVersion(value: unknown, requested: string): string {
  if (typeof value === "string" && value.length > 0) {
    return value;
  }
  if (Array.isArray(value)) {
    const versions = value.filter((entry): entry is string => typeof entry === "string");
    const last = versions.at(-1);
    if (last !== undefined) {
      return last;
    }
  }
  throw new NopmError(
    "NP0011",
    `npm did not return a concrete version for '${requested}'.`,
    requested,
  );
}

function selectMetadata(
  value: unknown,
  version: string,
  requested: string,
): Record<string, unknown> {
  if (isRecord(value)) {
    return value;
  }
  if (Array.isArray(value)) {
    const match = value.find(
      (entry) => isRecord(entry) && entry["version"] === version,
    );
    if (isRecord(match)) {
      return match;
    }
  }
  throw new NopmError(
    "NP0011",
    `npm did not return metadata for '${requested}'.`,
    requested,
  );
}

function stringField(value: Record<string, unknown>, key: string): string | null {
  const entry = value[key];
  return typeof entry === "string" && entry.length > 0 ? entry : null;
}

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}
