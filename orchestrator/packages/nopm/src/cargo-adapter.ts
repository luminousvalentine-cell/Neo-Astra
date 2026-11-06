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

const EXACT_SEMVER =
  /^(0|[1-9]\d*)\.(0|[1-9]\d*)\.(0|[1-9]\d*)(?:-[0-9A-Za-z-]+(?:\.[0-9A-Za-z-]+)*)?(?:\+[0-9A-Za-z-]+(?:\.[0-9A-Za-z-]+)*)?$/;
const CRATES_IO = "https://crates.io";
const USER_AGENT = "neo-astra-nopm/0.1.0";

export interface CargoAdapterOptions {
  readonly fetchImpl?: typeof fetch;
  readonly registry?: string;
}

export async function resolveCargoPackage(
  id: PackageId,
  alias: string,
  options: CargoAdapterOptions = {},
): Promise<AdapterResolution> {
  if (id.source !== "cargo") {
    throw new NopmError("NP0011", "Expected a Cargo package ID.", formatPackageId(id));
  }

  const fetchImpl = options.fetchImpl ?? fetch;
  const registry = options.registry ?? CRATES_IO;
  let version = id.version;

  if (version === null || !EXACT_SEMVER.test(version)) {
    const crate = await fetchJson(
      fetchImpl,
      `${registry}/api/v1/crates/${encodeURIComponent(id.name)}`,
      `read crates.io metadata for '${id.name}'`,
    );
    version = resolveCargoVersion(id.name, version, crate);
  }

  const details = await fetchJson(
    fetchImpl,
    `${registry}/api/v1/crates/${encodeURIComponent(id.name)}/${encodeURIComponent(version)}`,
    `read crates.io metadata for '${id.name}@${version}'`,
  );
  const versionRecord = isRecord(details["version"]) ? details["version"] : details;
  const returnedVersion = stringField(versionRecord, "num");
  if (returnedVersion !== version) {
    throw new NopmError(
      "NP0011",
      `crates.io returned version '${returnedVersion ?? "<missing>"}' for '${id.name}@${version}'.`,
      `${id.name}@${version}`,
    );
  }
  if (versionRecord["yanked"] === true) {
    throw new NopmError(
      "NP0011",
      `Cargo package '${id.name}@${version}' is yanked.`,
      `${id.name}@${version}`,
    );
  }

  const checksum = stringField(versionRecord, "checksum");
  if (checksum === null) {
    throw new NopmError(
      "NP0011",
      `crates.io metadata for '${id.name}@${version}' is missing a checksum.`,
      `${id.name}@${version}`,
    );
  }
  const metadata: Record<string, unknown> = {};
  const license = stringField(versionRecord, "license");
  if (license !== null) {
    metadata["license"] = license;
  }
  const rustVersion = stringField(versionRecord, "rust_version");
  if (rustVersion !== null) {
    metadata["rustVersion"] = rustVersion;
  }
  const links = stringField(versionRecord, "links");
  if (links !== null) {
    metadata["links"] = links;
  }
  if (isRecord(versionRecord["features"])) {
    metadata["features"] = versionRecord["features"];
  }

  const dependencies = await fetchJson(
    fetchImpl,
    `${registry}/api/v1/crates/${encodeURIComponent(id.name)}/${encodeURIComponent(version)}/dependencies`,
    `read crates.io dependencies for '${id.name}@${version}'`,
  );
  const dependencyMap: Record<string, string> = {};
  const rawDependencies = dependencies["dependencies"];
  if (Array.isArray(rawDependencies)) {
    for (const dependency of rawDependencies) {
      if (!isRecord(dependency)) {
        continue;
      }
      const kind = stringField(dependency, "kind") ?? "normal";
      if (kind !== "normal" || dependency["optional"] === true) {
        continue;
      }
      const crate = stringField(dependency, "package") ?? stringField(dependency, "crate_id");
      const requirement = stringField(dependency, "req");
      if (crate !== null && requirement !== null) {
        dependencyMap[crate] = requirement;
      }
    }
  }

  const exactId = parsePackageId(`cargo:${id.name}@${version}`);
  const descriptor = createDescriptor({
    id: exactId,
    dependencies: dependencyMap,
    integrity: `sha256:${checksum}`,
    resolved: `https://static.crates.io/crates/${id.name}/${id.name}-${version}.crate`,
    registry,
    lifecycleScripts: false,
    ...(Object.keys(metadata).length === 0 ? {} : { metadata }),
  });

  return Object.freeze({
    descriptor,
    request: parseDependencyRequest(alias, formatPackageId(exactId)),
  });
}

function resolveCargoVersion(
  name: string,
  requirement: string | null,
  crateMetadata: Record<string, unknown>,
): string {
  const versions = Array.isArray(crateMetadata["versions"])
    ? crateMetadata["versions"].filter(isRecord)
    : [];
  const candidates = versions
    .map((entry) => ({
      text: stringField(entry, "num"),
      yanked: entry["yanked"] === true,
    }))
    .filter(
      (entry): entry is { text: string; yanked: boolean } =>
        entry.text !== null && !entry.yanked,
    )
    .map((entry) => ({ version: parseSemVer(entry.text), text: entry.text }))
    .filter(
      (entry): entry is { version: SemVer; text: string } => entry.version !== null,
    );

  if (requirement === null) {
    const crateRecord = isRecord(crateMetadata["crate"]) ? crateMetadata["crate"] : crateMetadata;
    const maxStable = stringField(crateRecord, "max_stable_version");
    if (maxStable !== null) {
      return maxStable;
    }
    const stable = candidates
      .filter((entry) => entry.version.pre === null)
      .sort((left, right) => compareSemVer(right.version, left.version));
    const selected = stable[0] ?? candidates.sort((left, right) => compareSemVer(right.version, left.version))[0];
    if (selected === undefined) {
      throw new NopmError("NP0011", `crates.io has no usable versions for '${name}'.`, name);
    }
    return selected.text;
  }

  const matcher = requirementMatcher(requirement);
  const selected = candidates
    .filter((entry) => matcher(entry.version))
    .sort((left, right) => compareSemVer(right.version, left.version))[0];
  if (selected === undefined) {
    throw new NopmError(
      "NP0011",
      `No crates.io version of '${name}' satisfies '${requirement}'.`,
      `${name}@${requirement}`,
    );
  }
  return selected.text;
}

interface SemVer {
  readonly major: number;
  readonly minor: number;
  readonly patch: number;
  readonly pre: string | null;
}

function parseSemVer(text: string): SemVer | null {
  const match = /^(0|[1-9]\d*)(?:\.(0|[1-9]\d*))?(?:\.(0|[1-9]\d*))?(?:-([0-9A-Za-z.-]+))?(?:\+[0-9A-Za-z.-]+)?$/.exec(
    text,
  );
  if (match === null) {
    return null;
  }
  return {
    major: Number(match[1] ?? 0),
    minor: Number(match[2] ?? 0),
    patch: Number(match[3] ?? 0),
    pre: match[4] ?? null,
  };
}

function compareSemVer(left: SemVer, right: SemVer): number {
  if (left.major !== right.major) {
    return left.major - right.major;
  }
  if (left.minor !== right.minor) {
    return left.minor - right.minor;
  }
  if (left.patch !== right.patch) {
    return left.patch - right.patch;
  }
  if (left.pre === right.pre) {
    return 0;
  }
  if (left.pre === null) {
    return 1;
  }
  if (right.pre === null) {
    return -1;
  }
  return left.pre.localeCompare(right.pre);
}

function requirementMatcher(requirement: string): (version: SemVer) => boolean {
  const parts = requirement
    .split(/[,\s]+/)
    .map((part) => part.trim())
    .filter((part) => part.length > 0);
  const matchers = parts.map(comparatorMatcher);
  return (version) => {
    if (version.pre !== null && !requirement.includes("-")) {
      return false;
    }
    return matchers.every((matcher) => matcher(version));
  };
}

function comparatorMatcher(comparator: string): (version: SemVer) => boolean {
  if (comparator === "*") {
    return () => true;
  }
  for (const operator of [">=", "<=", ">", "<", "="] as const) {
    if (comparator.startsWith(operator)) {
      const version = parseSemVer(comparator.slice(operator.length).trim());
      if (version === null) {
        throw new NopmError("NP0011", `Invalid Cargo version requirement '${comparator}'.`, comparator);
      }
      return (candidate) => {
        const comparison = compareSemVer(candidate, version);
        switch (operator) {
          case ">=":
            return comparison >= 0;
          case "<=":
            return comparison <= 0;
          case ">":
            return comparison > 0;
          case "<":
            return comparison < 0;
          case "=":
            return comparison === 0;
        }
      };
    }
  }

  if (comparator.startsWith("^")) {
    return caretMatcher(parseRequiredVersion(comparator.slice(1)));
  }
  if (comparator.startsWith("~")) {
    return tildeMatcher(parseRequiredVersion(comparator.slice(1)));
  }
  if (comparator.includes("*")) {
    return wildcardMatcher(comparator);
  }
  return caretMatcher(parseRequiredVersion(comparator));
}

function parseRequiredVersion(text: string): SemVer {
  const version = parseSemVer(text);
  if (version === null) {
    throw new NopmError("NP0011", `Invalid Cargo version requirement '${text}'.`, text);
  }
  return version;
}

function caretMatcher(version: SemVer): (candidate: SemVer) => boolean {
  let upper: SemVer;
  if (version.major > 0) {
    upper = { major: version.major + 1, minor: 0, patch: 0, pre: null };
  } else if (version.minor > 0) {
    upper = { major: 0, minor: version.minor + 1, patch: 0, pre: null };
  } else {
    upper = { major: 0, minor: 0, patch: version.patch + 1, pre: null };
  }
  return (candidate) =>
    compareSemVer(candidate, version) >= 0 && compareSemVer(candidate, upper) < 0;
}

function tildeMatcher(version: SemVer): (candidate: SemVer) => boolean {
  const upper: SemVer =
    version.major > 0 || version.minor === 0
      ? { major: version.major + 1, minor: 0, patch: 0, pre: null }
      : { major: 0, minor: version.minor + 1, patch: 0, pre: null };
  return (candidate) =>
    compareSemVer(candidate, version) >= 0 && compareSemVer(candidate, upper) < 0;
}

function wildcardMatcher(requirement: string): (candidate: SemVer) => boolean {
  const [majorText, minorText] = requirement.split(".");
  const major = Number(majorText);
  if (!Number.isInteger(major)) {
    throw new NopmError("NP0011", `Invalid Cargo wildcard '${requirement}'.`, requirement);
  }
  if (minorText === "*" || minorText === undefined) {
    return (candidate) => candidate.major === major;
  }
  const minor = Number(minorText);
  if (!Number.isInteger(minor)) {
    throw new NopmError("NP0011", `Invalid Cargo wildcard '${requirement}'.`, requirement);
  }
  return (candidate) => candidate.major === major && candidate.minor === minor;
}

async function fetchJson(
  fetchImpl: typeof fetch,
  url: string,
  action: string,
): Promise<Record<string, unknown>> {  let response: Response;
  try {
    response = await fetchImpl(url, {
      headers: {
        "User-Agent": USER_AGENT,
        Accept: "application/json",
      },
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
