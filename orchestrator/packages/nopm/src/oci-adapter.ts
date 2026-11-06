import {
  NopmError,
  createDescriptor,
  formatPackageId,
  parseDependencyRequest,
  parsePackageId,
  type PackageId,
} from "./index.js";
import type { AdapterResolution } from "./npm-adapter.js";

const USER_AGENT = "neo-astra-nopm/0.1.0";

export interface OciAdapterOptions {
  readonly fetchImpl?: typeof fetch;
}

export async function resolveOciPackage(
  id: PackageId,
  alias: string,
  options: OciAdapterOptions = {},
): Promise<AdapterResolution> {
  if (id.source !== "oci" || id.reference === null) {
    throw new NopmError("NP0011", "Expected an OCI package ID with a reference.", formatPackageId(id));
  }

  const fetchImpl = options.fetchImpl ?? fetch;
  const slash = id.name.indexOf("/");
  if (slash <= 0 || slash === id.name.length - 1) {
    throw new NopmError("NP0011", "OCI package names require registry/repository.", id.name);
  }
  const registry = id.name.slice(0, slash);
  const repository = id.name.slice(slash + 1);
  const base = `${registry.startsWith("localhost") || registry.includes(":") ? "http" : "https"}://${registry}`;
  const manifestUrl = `${base}/v2/${repository}/manifests/${encodeURIComponent(id.reference)}`;

  let response: Response;
  try {
    response = await fetchImpl(manifestUrl, {
      headers: {
        "User-Agent": USER_AGENT,
        Accept: "application/vnd.oci.image.manifest.v1+json, application/vnd.docker.distribution.manifest.v2+json",
      },
    });
  } catch (error) {
    throw new NopmError(
      "NP0011",
      `Unable to read OCI manifest for '${formatPackageId(id)}': ${error instanceof Error ? error.message : String(error)}`,
      manifestUrl,
      { cause: error },
    );
  }
  if (!response.ok) {
    throw new NopmError("NP0011", `Unable to read OCI manifest: HTTP ${response.status}.`, manifestUrl);
  }

  let value: unknown;
  try {
    value = await response.json();
  } catch (error) {
    throw new NopmError("NP0011", "OCI registry returned invalid manifest JSON.", manifestUrl, {
      cause: error,
    });
  }
  if (!isRecord(value) || !Array.isArray(value["layers"])) {
    throw new NopmError("NP0011", "OCI manifest has no layers.", manifestUrl);
  }

  const layer = value["layers"].find((entry) => {
    if (!isRecord(entry)) {
      return false;
    }
    const mediaType = typeof entry["mediaType"] === "string" ? entry["mediaType"] : "";
    return mediaType.includes("tar") || mediaType.includes("neoastra");
  });
  if (layer === undefined || !isRecord(layer)) {
    throw new NopmError("NP0011", "OCI package has no distributable tar layer.", manifestUrl);
  }

  const digest = typeof layer["digest"] === "string" ? layer["digest"] : null;
  if (digest === null || !/^(sha256|sha512):[0-9a-f]+$/i.test(digest)) {
    throw new NopmError("NP0011", "OCI layer has no valid content digest.", manifestUrl);
  }
  const integrity = digest.toLowerCase();
  const resolved = `${base}/v2/${repository}/blobs/${digest}`;
  const mediaType = typeof layer["mediaType"] === "string" ? layer["mediaType"] : null;
  const exactId = parsePackageId(`oci:${id.name}@${id.reference}`);
  const descriptor = createDescriptor({
    id: exactId,
    dependencies: {},
    integrity,
    resolved,
    registry: base,
    lifecycleScripts: false,
    ...(mediaType === null ? {} : { metadata: { mediaType } }),
  });

  return Object.freeze({
    descriptor,
    request: parseDependencyRequest(alias, formatPackageId(exactId)),
  });
}

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}
