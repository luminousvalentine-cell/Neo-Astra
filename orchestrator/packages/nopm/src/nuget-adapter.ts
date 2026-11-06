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
import type { AdapterResolution } from "./npm-adapter.js";

const NUGET = "https://api.nuget.org";
const USER_AGENT = "neo-astra-nopm/0.1.0";

export interface NuGetAdapterOptions {
  readonly fetchImpl?: typeof fetch;
  readonly registry?: string;
}

export async function resolveNuGetPackage(
  id: PackageId,
  alias: string,
  options: NuGetAdapterOptions = {},
): Promise<AdapterResolution> {
  if (id.source !== "nuget") {
    throw new NopmError("NP0011", "Expected a NuGet package ID.", formatPackageId(id));
  }

  const fetchImpl = options.fetchImpl ?? fetch;
  const registry = options.registry ?? NUGET;
  const lower = id.name.toLowerCase();
  const version = id.version ?? (await latestVersion(fetchImpl, registry, lower));
  const nupkg = `${registry}/v3-flatcontainer/${encodeURIComponent(lower)}/${encodeURIComponent(version)}/${encodeURIComponent(lower)}.${encodeURIComponent(version)}.nupkg`;

  const integrity = await fetchIntegrity(fetchImpl, `${nupkg}.sha512`);
  const dependencies = await fetchDependencies(fetchImpl, registry, lower, version);
  const exactId = parsePackageId(`nuget:${id.name}@${version}`);
  const descriptor = createDescriptor({
    id: exactId,
    dependencies,
    integrity,
    resolved: nupkg,
    registry,
    lifecycleScripts: false,
  });

  return Object.freeze({
    descriptor,
    request: parseDependencyRequest(alias, formatPackageId(exactId)),
  });
}

async function latestVersion(
  fetchImpl: typeof fetch,
  registry: string,
  lower: string,
): Promise<string> {
  const value = await fetchJson(
    fetchImpl,
    `${registry}/v3-flatcontainer/${encodeURIComponent(lower)}/index.json`,
    `read NuGet versions for '${lower}'`,
  );
  const versions = Array.isArray(value["versions"]) ? value["versions"] : [];
  const stable = versions
    .filter((entry): entry is string => typeof entry === "string" && !entry.includes("-"))
    .at(-1);
  if (stable === undefined) {
    throw new NopmError("NP0011", `NuGet did not return a stable version for '${lower}'.`, lower);
  }
  return stable;
}

async function fetchIntegrity(fetchImpl: typeof fetch, url: string): Promise<string> {
  const response = await request(fetchImpl, url, "read NuGet package integrity");
  if (!response.ok) {
    throw new NopmError("NP0011", `Unable to read NuGet integrity: HTTP ${response.status}.`, url);
  }
  const text = (await response.text()).trim();
  if (text.length === 0) {
    throw new NopmError("NP0011", "NuGet returned an empty integrity hash.", url);
  }
  return `sha512-${text}`;
}

async function fetchDependencies(
  fetchImpl: typeof fetch,
  registry: string,
  lower: string,
  version: string,
): Promise<Record<string, string>> {
  const url = `${registry}/v3/registration5-gz-semver2/${encodeURIComponent(lower)}/${encodeURIComponent(version)}.json`;
  const value = await fetchJson(fetchImpl, url, `read NuGet dependencies for '${lower}@${version}'`);
  const catalog = isRecord(value["catalogEntry"]) ? value["catalogEntry"] : value;
  const groups = Array.isArray(catalog["dependencyGroups"])
    ? catalog["dependencyGroups"]
    : [];
  const dependencies: Record<string, string> = {};

  for (const group of groups) {
    if (!isRecord(group) || !Array.isArray(group["dependencies"])) {
      continue;
    }
    for (const dependency of group["dependencies"]) {
      if (!isRecord(dependency)) {
        continue;
      }
      const name = stringField(dependency, "id");
      const range = stringField(dependency, "range");
      if (name === null || range === null) {
        continue;
      }
      const exact = exactRange(range);
      if (exact !== null) {
        dependencies[name] = `nuget:${name}@${exact}`;
      }
    }
  }

  return dependencies;
}

function exactRange(range: string): string | null {
  const match = /^\[\s*([^,\s]+)\s*\]$/.exec(range);
  return match?.[1] ?? null;
}

async function fetchJson(
  fetchImpl: typeof fetch,
  url: string,
  action: string,
): Promise<Record<string, unknown>> {
  const response = await request(fetchImpl, url, action);
  if (!response.ok) {
    throw new NopmError("NP0011", `Unable to ${action}: HTTP ${response.status}.`, url);
  }
  let value: unknown;
  try {
    value = await response.json();
  } catch (error) {
    throw new NopmError("NP0011", `Unable to parse JSON while trying to ${action}.`, url, {
      cause: error,
    });
  }
  if (!isRecord(value)) {
    throw new NopmError("NP0011", `Unexpected JSON while trying to ${action}.`, url);
  }
  return value;
}

async function request(
  fetchImpl: typeof fetch,
  url: string,
  action: string,
): Promise<Response> {
  try {
    return await fetchImpl(url, {
      headers: { "User-Agent": USER_AGENT, Accept: "application/json" },
    });
  } catch (error) {
    throw new NopmError("NP0011", `Unable to ${action}: ${error instanceof Error ? error.message : String(error)}`, url, {
      cause: error,
    });
  }
}

function stringField(value: Record<string, unknown>, key: string): string | null {
  const entry = value[key];
  return typeof entry === "string" && entry.length > 0 ? entry : null;
}

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}
