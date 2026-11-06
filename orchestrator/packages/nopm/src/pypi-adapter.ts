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

const PYPI = "https://pypi.org";
const USER_AGENT = "neo-astra-nopm/0.1.0";

export interface PyPiAdapterOptions {
  readonly fetchImpl?: typeof fetch;
  readonly registry?: string;
}

export async function resolvePyPiPackage(
  id: PackageId,
  alias: string,
  options: PyPiAdapterOptions = {},
): Promise<AdapterResolution> {
  if (id.source !== "pypi") {
    throw new NopmError("NP0011", "Expected a PyPI package ID.", formatPackageId(id));
  }

  const fetchImpl = options.fetchImpl ?? fetch;
  const registry = options.registry ?? PYPI;
  const version = id.version ?? (await latestVersion(fetchImpl, registry, id.name));
  const url = `${registry}/pypi/${encodeURIComponent(id.name)}/${encodeURIComponent(version)}/json`;
  const value = await fetchJson(fetchImpl, url, `read PyPI metadata for '${id.name}@${version}'`);
  const info = isRecord(value["info"]) ? value["info"] : null;
  if (info === null) {
    throw new NopmError("NP0011", `PyPI did not return info for '${id.name}@${version}'.`, url);
  }

  const returnedName = stringField(info, "name") ?? id.name;
  const returnedVersion = stringField(info, "version");
  if (returnedVersion !== version) {
    throw new NopmError(
      "NP0011",
      `PyPI returned version '${returnedVersion ?? "<missing>"}' for '${id.name}@${version}'.`,
      url,
    );
  }

  const files = Array.isArray(value["urls"]) ? value["urls"].filter(isRecord) : [];
  const artifact =
    files.find((entry) => entry["packagetype"] === "sdist") ??
    files.find((entry) => entry["packagetype"] === "bdist_wheel") ??
    files[0];
  if (artifact === undefined) {
    throw new NopmError("NP0011", `PyPI has no distribution for '${id.name}@${version}'.`, url);
  }
  const resolved = stringField(artifact, "url");
  const digests = isRecord(artifact["digests"]) ? artifact["digests"] : null;
  const sha256 = digests === null ? null : stringField(digests, "sha256");
  if (resolved === null || sha256 === null) {
    throw new NopmError(
      "NP0011",
      `PyPI distribution for '${id.name}@${version}' is missing a URL or sha256.`,
      url,
    );
  }

  const dependencies: Record<string, string> = {};
  const requirements = info["requires_dist"];
  if (Array.isArray(requirements)) {
    for (const requirement of requirements) {
      if (typeof requirement !== "string") {
        continue;
      }
      const parsed = parseRequirement(requirement);
      if (parsed !== null) {
        dependencies[parsed.name] = parsed.spec;
      }
    }
  }

  const exactId = parsePackageId(`pypi:${returnedName}@${version}`);
  const descriptor = createDescriptor({
    id: exactId,
    dependencies,
    integrity: `sha256:${sha256}`,
    resolved,
    registry,
    lifecycleScripts: false,
    ...(stringField(artifact, "packagetype") === null
      ? {}
      : { metadata: { packagetype: stringField(artifact, "packagetype") } }),
  });

  return Object.freeze({
    descriptor,
    request: parseDependencyRequest(alias, formatPackageId(exactId)),
  });
}

async function latestVersion(
  fetchImpl: typeof fetch,
  registry: string,
  name: string,
): Promise<string> {
  const url = `${registry}/pypi/${encodeURIComponent(name)}/json`;
  const value = await fetchJson(fetchImpl, url, `read PyPI metadata for '${name}'`);
  const info = isRecord(value["info"]) ? value["info"] : null;
  const version = info === null ? null : stringField(info, "version");
  if (version === null) {
    throw new NopmError("NP0011", `PyPI did not return a latest version for '${name}'.`, url);
  }
  return version;
}

function parseRequirement(requirement: string): { name: string; spec: string } | null {
  const withoutMarker = requirement.split(";")[0]?.trim() ?? "";
  const match = /^([A-Za-z0-9._-]+)\s*(.*)$/.exec(withoutMarker);
  if (match === null) {
    return null;
  }
  const name = (match[1] ?? "").replace(/_/g, "-").toLowerCase();
  const requirementText = match[2] ?? "";
  const exact = /(?:^|[, ])==\s*([A-Za-z0-9._+!-]+)/.exec(requirementText);
  if (exact !== null) {
    return { name, spec: `pypi:${name}@${exact[1]}` };
  }
  if (requirementText.trim().length === 0) {
    return { name, spec: `pypi:${name}` };
  }
  return null;
}

async function fetchJson(
  fetchImpl: typeof fetch,
  url: string,
  action: string,
): Promise<Record<string, unknown>> {
  let response: Response;
  try {
    response = await fetchImpl(url, {
      headers: { "User-Agent": USER_AGENT, Accept: "application/json" },
    });
  } catch (error) {
    throw new NopmError("NP0011", `Unable to ${action}: ${error instanceof Error ? error.message : String(error)}`, url, {
      cause: error,
    });
  }
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

function stringField(value: Record<string, unknown>, key: string): string | null {
  const entry = value[key];
  return typeof entry === "string" && entry.length > 0 ? entry : null;
}

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}
