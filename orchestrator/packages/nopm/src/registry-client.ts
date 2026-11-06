import {
  NopmError,
  createDescriptor,
  formatPackageId,
  parseDependencyRequest,
  parsePackageId,
  type PackageId,
} from "./index.js";
import type { AdapterResolution } from "./npm-adapter.js";
import { verifyRegistryPackage, type PublishRequest } from "@neo-astra/registry";

const USER_AGENT = "neo-astra-nopm/0.1.0";

export interface RegistryAdapterOptions {
  readonly registry: string;
  readonly fetchImpl?: typeof fetch;
  readonly token?: string;
  readonly requireSignature?: boolean;
  readonly publicKey?: string;
}

export async function resolveRegistryPackage(
  id: PackageId,
  alias: string,
  options: RegistryAdapterOptions,
): Promise<AdapterResolution> {
  if (id.source !== "nova") {
    throw new NopmError("NP0011", "Expected a Nova registry package ID.", formatPackageId(id));
  }
  if (id.version === null) {
    throw new NopmError(
      "NP0011",
      "Nova registry packages require an exact version.",
      formatPackageId(id),
    );
  }

  const url = `${options.registry.replace(/\/+$/, "")}/v1/packages/${encodeURIComponent(id.name)}/${encodeURIComponent(id.version)}`;
  const response = await fetchRegistry(options, url);
  if (!response.ok) {
    throw new NopmError(
      "NP0011",
      `Unable to resolve '${formatPackageId(id)}': HTTP ${response.status}.`,
      url,
    );
  }

  let value: unknown;
  try {
    value = await response.json();
  } catch (error) {
    throw new NopmError("NP0011", "Registry returned invalid JSON.", url, { cause: error });
  }
  if (!isRecord(value)) {
    throw new NopmError("NP0011", "Registry returned an invalid package descriptor.", url);
  }

  const version = stringField(value, "version") ?? id.version;
  if (version !== id.version) {
    throw new NopmError(
      "NP0011",
      `Registry returned version '${version}' for '${formatPackageId(id)}'.`,
      url,
    );
  }
  const integrity = stringField(value, "integrity");
  if (integrity === null) {
    throw new NopmError(
      "NP0011",
      `Registry descriptor for '${formatPackageId(id)}' is missing integrity.`,
      url,
    );
  }

  const dependencies: Record<string, string> = {};
  if (isRecord(value["dependencies"])) {
    for (const [dependency, spec] of Object.entries(value["dependencies"])) {
      if (typeof spec === "string") {
        dependencies[dependency] = spec;
      }
    }
  }

  const exactId = parsePackageId(`nova:${id.name}@${version}`);
  const targets = stringArray(value["targets"]);
  const capabilities = stringArray(value["capabilities"]);
  const resolved = stringField(value, "resolved") ?? url;
  const lifecycleScripts = value["lifecycleScripts"] === true;
  const signature = stringField(value, "signature");
  const publicKey = options.publicKey ?? stringField(value, "publicKey");

  if (options.requireSignature === true && (signature === null || publicKey === null)) {
    throw new NopmError(
      "NP0011",
      `Registry descriptor for '${formatPackageId(id)}' is not signed.`,
      url,
    );
  }
  if (signature !== null && publicKey !== null) {
    const payload: PublishRequest = {
      name: id.name,
      version,
      dependencies,
      targets,
      capabilities,
      integrity,
      resolved,
      lifecycleScripts,
    };
    if (!verifyRegistryPackage(payload, signature, publicKey)) {
      throw new NopmError(
        "NP0011",
        `Registry signature verification failed for '${formatPackageId(id)}'.`,
        url,
      );
    }
  }

  const metadata = isRecord(value["metadata"]) ? value["metadata"] : undefined;
  const descriptor = createDescriptor({
    id: exactId,
    dependencies,
    targets,
    capabilities,
    integrity,
    resolved,
    registry: options.registry,
    lifecycleScripts,
    ...(metadata === undefined ? {} : { metadata }),
  });

  return Object.freeze({
    descriptor,
    request: parseDependencyRequest(alias, formatPackageId(exactId)),
  });
}

async function fetchRegistry(
  options: RegistryAdapterOptions,
  url: string,
): Promise<Response> {
  const fetchImpl = options.fetchImpl ?? fetch;
  try {
    return await fetchImpl(url, {
      headers: {
        "User-Agent": USER_AGENT,
        Accept: "application/json",
        ...(options.token === undefined ? {} : { Authorization: `Bearer ${options.token}` }),
      },
    });
  } catch (error) {
    throw new NopmError(
      "NP0011",
      `Unable to contact registry: ${error instanceof Error ? error.message : String(error)}`,
      url,
      { cause: error },
    );
  }
}

function stringField(value: Record<string, unknown>, key: string): string | null {
  const entry = value[key];
  return typeof entry === "string" && entry.length > 0 ? entry : null;
}

function stringArray(value: unknown): string[] {
  if (!Array.isArray(value)) {
    return [];
  }
  return value.filter((entry): entry is string => typeof entry === "string");
}

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}
