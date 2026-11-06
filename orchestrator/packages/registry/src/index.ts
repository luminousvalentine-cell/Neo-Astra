import { generateKeyPairSync, sign, verify } from "node:crypto";
import { createServer, type IncomingMessage, type Server, type ServerResponse } from "node:http";

export type RegistryErrorCode = "NR0001" | "NR0002" | "NR0003" | "NR0004";

export class RegistryError extends Error {
  readonly code: RegistryErrorCode;
  readonly status: number;

  constructor(code: RegistryErrorCode, message: string, status: number) {
    super(message);
    this.name = "RegistryError";
    this.code = code;
    this.status = status;
  }
}

export interface RegistryPackageVersion {
  readonly name: string;
  readonly version: string;
  readonly dependencies: Readonly<Record<string, string>>;
  readonly targets: readonly string[];
  readonly capabilities: readonly string[];
  readonly integrity: string;
  readonly resolved: string;
  readonly lifecycleScripts: boolean;
  readonly yanked: boolean;
  readonly signature?: string;
  readonly publicKey?: string;
  readonly metadata?: Readonly<Record<string, unknown>>;
}

export interface PublishRequest {
  readonly name: string;
  readonly version: string;
  readonly dependencies?: Readonly<Record<string, string>>;
  readonly targets?: readonly string[];
  readonly capabilities?: readonly string[];
  readonly integrity: string;
  readonly resolved: string;
  readonly lifecycleScripts?: boolean;
  readonly signature?: string;
  readonly publicKey?: string;
  readonly metadata?: Readonly<Record<string, unknown>>;
}

export interface RegistryServerOptions {
  readonly upstreams?: readonly string[];
  readonly fetchImpl?: typeof fetch;
  readonly requireSignature?: boolean;
  readonly trustedPublicKey?: string;
}

export interface RegistryAudit {
  readonly name: string;
  readonly version: string;
  readonly yanked: boolean;
  readonly advisories: readonly unknown[];
}

const SEMVER =
  /^(0|[1-9]\d*)\.(0|[1-9]\d*)\.(0|[1-9]\d*)(?:-[0-9A-Za-z-]+(?:\.[0-9A-Za-z-]+)*)?(?:\+[0-9A-Za-z-]+(?:\.[0-9A-Za-z-]+)*)?$/;
const MAX_BODY = 1024 * 1024;

export class RegistryStore {
  readonly #packages = new Map<string, Map<string, RegistryPackageVersion>>();

  publish(request: PublishRequest): RegistryPackageVersion {
    validatePackage(request);
    const versions = this.#packages.get(request.name) ?? new Map();
    if (versions.has(request.version)) {
      throw new RegistryError(
        "NR0001",
        `Package '${request.name}@${request.version}' already exists.`,
        409,
      );
    }

    const packageVersion: RegistryPackageVersion = Object.freeze({
      name: request.name,
      version: request.version,
      dependencies: Object.freeze({ ...(request.dependencies ?? {}) }),
      targets: Object.freeze([...(request.targets ?? [])]),
      capabilities: Object.freeze([...(request.capabilities ?? [])]),
      integrity: request.integrity,
      resolved: request.resolved,
      lifecycleScripts: request.lifecycleScripts ?? false,
      yanked: false,
      ...(request.signature === undefined ? {} : { signature: request.signature }),
      ...(request.publicKey === undefined ? {} : { publicKey: request.publicKey }),
      ...(request.metadata === undefined
        ? {}
        : { metadata: Object.freeze({ ...request.metadata }) }),
    });
    versions.set(request.version, packageVersion);
    this.#packages.set(request.name, versions);
    return packageVersion;
  }

  get(name: string, version: string): RegistryPackageVersion {
    const packageVersion = this.#packages.get(name)?.get(version);
    if (packageVersion === undefined) {
      throw new RegistryError("NR0003", `Package '${name}@${version}' was not found.`, 404);
    }
    return packageVersion;
  }

  versions(name: string): readonly RegistryPackageVersion[] {
    const versions = this.#packages.get(name);
    if (versions === undefined) {
      throw new RegistryError("NR0003", `Package '${name}' was not found.`, 404);
    }
    return [...versions.values()].sort((left, right) =>
      left.version.localeCompare(right.version),
    );
  }

  search(query: string): readonly RegistryPackageVersion[] {
    const normalized = query.toLowerCase();
    const results: RegistryPackageVersion[] = [];
    for (const versions of this.#packages.values()) {
      const latest = [...versions.values()].sort((left, right) =>
        right.version.localeCompare(left.version),
      )[0];
      if (latest !== undefined && latest.name.toLowerCase().includes(normalized)) {
        results.push(latest);
      }
    }
    return results.sort((left, right) => left.name.localeCompare(right.name));
  }

  yank(name: string, version: string): RegistryPackageVersion {
    const current = this.get(name, version);
    if (current.yanked) {
      return current;
    }
    const versions = this.#packages.get(name);
    if (versions === undefined) {
      throw new RegistryError("NR0003", `Package '${name}' was not found.`, 404);
    }
    const updated: RegistryPackageVersion = Object.freeze({ ...current, yanked: true });
    versions.set(version, updated);
    return updated;
  }

  audit(name: string, version: string): RegistryAudit {
    const packageVersion = this.get(name, version);
    return Object.freeze({
      name,
      version,
      yanked: packageVersion.yanked,
      advisories: Object.freeze([]),
    });
  }
}

export function createRegistryServer(
  store = new RegistryStore(),
  options: RegistryServerOptions = {},
): Server {
  return createServer((request, response) => {
    void handle(store, options, request, response);
  });
}

export async function startRegistryServer(
  store = new RegistryStore(),
  port = 0,
  host = "127.0.0.1",
  options: RegistryServerOptions = {},
): Promise<{ readonly server: Server; readonly url: string }> {
  const server = createRegistryServer(store, options);
  await new Promise<void>((resolve, reject) => {
    server.once("error", reject);
    server.listen(port, host, () => resolve());
  });
  const address = server.address();
  if (address === null || typeof address === "string") {
    throw new Error("Registry server did not expose a TCP address.");
  }
  return { server, url: `http://${host}:${address.port}` };
}

async function handle(
  store: RegistryStore,
  options: RegistryServerOptions,
  request: IncomingMessage,
  response: ServerResponse,
): Promise<void> {
  try {
    const url = new URL(request.url ?? "/", "http://registry.local");
    const segments = url.pathname
      .split("/")
      .filter((segment) => segment.length > 0)
      .map((segment) => decodeURIComponent(segment));

    if (segments[0] !== "v1") {
      throw new RegistryError("NR0002", "Unknown registry route.", 404);
    }

    if (request.method === "GET" && segments[1] === "packages") {
      if (segments.length === 3) {
        const versions = store.versions(segments[2] ?? "");
        sendJson(response, 200, {
          name: segments[2],
          versions: versions.map((entry) => ({
            version: entry.version,
            yanked: entry.yanked,
          })),
        });
        return;
      }
      if (segments.length === 4) {
        const name = segments[2] ?? "";
        const version = segments[3] ?? "";
        let entry: RegistryPackageVersion;
        try {
          entry = store.get(name, version);
        } catch (error) {
          if (!(error instanceof RegistryError) || error.code !== "NR0003") {
            throw error;
          }
          entry = await fetchUpstream(store, options, name, version);
        }
        if (entry.yanked) {
          throw new RegistryError("NR0004", `Package '${entry.name}@${entry.version}' is yanked.`, 410);
        }
        sendJson(response, 200, entry);
        return;
      }
    }

    if (request.method === "GET" && segments[1] === "search") {
      const query = url.searchParams.get("q") ?? "";
      sendJson(response, 200, { query, results: store.search(query) });
      return;
    }

    if (request.method === "GET" && segments[1] === "audit" && segments.length === 4) {
      sendJson(response, 200, store.audit(segments[2] ?? "", segments[3] ?? ""));
      return;
    }

    if (request.method === "POST" && segments[1] === "publish" && segments.length === 2) {
      const body = await readJson(request);
      const publishRequest = parsePublish(body);
      if (options.requireSignature === true) {
        const publicKey = options.trustedPublicKey ?? publishRequest.publicKey;
        if (publishRequest.signature === undefined || publicKey === undefined) {
          throw new RegistryError("NR0002", "Signed publishing requires signature and public key.", 401);
        }
        if (!verifyRegistryPackage(publishRequest, publishRequest.signature, publicKey)) {
          throw new RegistryError("NR0002", "Package signature verification failed.", 401);
        }
      }
      const published = store.publish(publishRequest);
      sendJson(response, 201, published);
      return;
    }

    if (request.method === "POST" && segments[1] === "yank" && segments.length === 2) {
      const body = await readJson(request);
      const name = requiredString(body, "name");
      const version = requiredString(body, "version");
      sendJson(response, 200, store.yank(name, version));
      return;
    }

    throw new RegistryError("NR0002", "Unknown registry route.", 404);
  } catch (error) {
    if (error instanceof RegistryError) {
      sendJson(response, error.status, { error: { code: error.code, message: error.message } });
      return;
    }
    sendJson(response, 500, {
      error: { code: "NR0002", message: error instanceof Error ? error.message : String(error) },
    });
  }
}

export function generateRegistryKeyPair(): { publicKey: string; privateKey: string } {
  const pair = generateKeyPairSync("ed25519");
  return {
    publicKey: pair.publicKey.export({ type: "spki", format: "pem" }).toString(),
    privateKey: pair.privateKey.export({ type: "pkcs8", format: "pem" }).toString(),
  };
}

export function signRegistryPackage(request: PublishRequest, privateKey: string): string {
  return sign(null, Buffer.from(registryPayload(request), "utf8"), privateKey).toString("base64");
}

export function verifyRegistryPackage(
  request: PublishRequest,
  signature: string,
  publicKey: string,
): boolean {
  try {
    return verify(
      null,
      Buffer.from(registryPayload(request), "utf8"),
      publicKey,
      Buffer.from(signature, "base64"),
    );
  } catch {
    return false;
  }
}

export function registryPayload(request: PublishRequest): string {
  const dependencies = Object.fromEntries(
    Object.entries(request.dependencies ?? {}).sort(([left], [right]) => left.localeCompare(right)),
  );
  return JSON.stringify({
    name: request.name,
    version: request.version,
    dependencies,
    targets: [...(request.targets ?? [])].sort(),
    capabilities: [...(request.capabilities ?? [])].sort(),
    integrity: request.integrity,
    resolved: request.resolved,
    lifecycleScripts: request.lifecycleScripts ?? false,
  });
}

async function fetchUpstream(
  store: RegistryStore,
  options: RegistryServerOptions,
  name: string,
  version: string,
): Promise<RegistryPackageVersion> {
  const upstreams = options.upstreams ?? [];
  const fetchImpl = options.fetchImpl ?? fetch;

  for (const upstream of upstreams) {
    const url = `${upstream.replace(/\/+$/, "")}/v1/packages/${encodeURIComponent(name)}/${encodeURIComponent(version)}`;
    try {
      const response = await fetchImpl(url, {
        headers: { Accept: "application/json" },
      });
      if (!response.ok) {
        continue;
      }
      const body = (await response.json()) as unknown;
      if (!isRecord(body)) {
        continue;
      }
      const request = parsePublish(body);
      try {
        store.publish(request);
      } catch (error) {
        if (!(error instanceof RegistryError) || error.code !== "NR0001") {
          throw error;
        }
      }
      return store.get(name, version);
    } catch {
      continue;
    }
  }

  throw new RegistryError("NR0003", `Package '${name}@${version}' was not found.`, 404);
}

function parsePublish(value: Record<string, unknown>): PublishRequest {
  const dependencies = optionalStringRecord(value, "dependencies");
  const targets = optionalStringArray(value, "targets");
  const capabilities = optionalStringArray(value, "capabilities");
  return {
    name: requiredString(value, "name"),
    version: requiredString(value, "version"),
    ...(dependencies === undefined ? {} : { dependencies }),
    ...(targets === undefined ? {} : { targets }),
    ...(capabilities === undefined ? {} : { capabilities }),
    integrity: requiredString(value, "integrity"),
    resolved: requiredString(value, "resolved"),
    lifecycleScripts: value["lifecycleScripts"] === true,
    ...(isRecord(value["metadata"]) ? { metadata: value["metadata"] } : {}),
    ...(typeof value["signature"] === "string" ? { signature: value["signature"] } : {}),
    ...(typeof value["publicKey"] === "string" ? { publicKey: value["publicKey"] } : {}),
  };
}

function validatePackage(request: PublishRequest): void {
  if (request.name.length === 0 || request.name.trim() !== request.name) {
    throw new RegistryError("NR0002", "Package name must be non-empty and trimmed.", 400);
  }
  if (!SEMVER.test(request.version)) {
    throw new RegistryError("NR0002", `Package version '${request.version}' is not exact semver.`, 400);
  }
  if (request.integrity.length === 0 || request.resolved.length === 0) {
    throw new RegistryError("NR0002", "Published packages require integrity and resolved URL.", 400);
  }
}

async function readJson(request: IncomingMessage): Promise<Record<string, unknown>> {
  const chunks: Buffer[] = [];
  let length = 0;
  for await (const chunk of request) {
    const buffer = Buffer.isBuffer(chunk) ? chunk : Buffer.from(chunk);
    length += buffer.length;
    if (length > MAX_BODY) {
      throw new RegistryError("NR0002", "Request body is too large.", 413);
    }
    chunks.push(buffer);
  }
  try {
    const value = JSON.parse(Buffer.concat(chunks).toString("utf8")) as unknown;
    if (!isRecord(value)) {
      throw new RegistryError("NR0002", "Request body must be a JSON object.", 400);
    }
    return value;
  } catch (error) {
    if (error instanceof RegistryError) {
      throw error;
    }
    throw new RegistryError("NR0002", "Request body is not valid JSON.", 400, );
  }
}

function sendJson(response: ServerResponse, status: number, value: unknown): void {
  const body = `${JSON.stringify(value)}\n`;
  response.writeHead(status, {
    "Content-Type": "application/json; charset=utf-8",
    "Content-Length": Buffer.byteLength(body),
  });
  response.end(body);
}

function requiredString(value: Record<string, unknown>, key: string): string {
  const entry = value[key];
  if (typeof entry !== "string" || entry.length === 0) {
    throw new RegistryError("NR0002", `'${key}' must be a non-empty string.`, 400);
  }
  return entry;
}

function optionalStringRecord(
  value: Record<string, unknown>,
  key: string,
): Readonly<Record<string, string>> | undefined {
  const entry = value[key];
  if (entry === undefined) {
    return undefined;
  }
  if (!isRecord(entry) || Object.values(entry).some((item) => typeof item !== "string")) {
    throw new RegistryError("NR0002", `'${key}' must be an object of strings.`, 400);
  }
  return entry as Record<string, string>;
}

function optionalStringArray(
  value: Record<string, unknown>,
  key: string,
): readonly string[] | undefined {
  const entry = value[key];
  if (entry === undefined) {
    return undefined;
  }
  if (!Array.isArray(entry) || entry.some((item) => typeof item !== "string")) {
    throw new RegistryError("NR0002", `'${key}' must be an array of strings.`, 400);
  }
  return entry as string[];
}

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}
