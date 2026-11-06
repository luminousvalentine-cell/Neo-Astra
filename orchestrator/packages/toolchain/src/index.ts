import { spawnSync } from "node:child_process";
import { createHash } from "node:crypto";
import { existsSync, mkdirSync, readFileSync, readdirSync, renameSync, rmSync, statSync, writeFileSync } from "node:fs";
import { join } from "node:path";

export class ToolchainError extends Error {
  readonly code: string;
  constructor(code: string, message: string) {
    super(message);
    this.name = "ToolchainError";
    this.code = code;
  }
}

export interface ToolchainPolicy {
  readonly request: "auto" | string;
}

export type ToolchainRung = "env" | "cache" | "path";

export interface ToolchainResolution {
  readonly rung: ToolchainRung;
  readonly id: string;
  readonly family: string;
  readonly path: string;
  readonly version: string | null;
}

export interface ToolchainProbe {
  readonly exists: (path: string) => boolean;
  readonly run: (command: string, args: readonly string[]) => { status: number | null; stdout: string; stderr: string };
  readonly readFileSync: (path: string) => string;
  readonly readdir: (path: string) => string[];
}

export interface ToolchainAttempt {
  readonly rung: ToolchainRung;
  readonly detail: string;
}

export interface ResolveOptions {
  readonly projectRoot: string;
  readonly policy: ToolchainPolicy;
  readonly env: { readonly ASTRA_CC?: string };
  readonly probe: ToolchainProbe;

  readonly onAttempt?: (attempt: ToolchainAttempt) => void;
}

const FAMILIES = new Set(["clang", "gcc", "zig", "cc"]);

function isRecord(v: unknown): v is Record<string, unknown> {
  return typeof v === "object" && v !== null && !Array.isArray(v);
}

export function parseToolchainPolicy(buildData: unknown): ToolchainPolicy {
  let raw: unknown = "auto";
  if (buildData !== undefined) {
    if (!isRecord(buildData)) {
      throw new ToolchainError("NS9101", "build manifest must be an object");
    }
    if (Object.prototype.hasOwnProperty.call(buildData, "toolchain")) {
      const tc = buildData["toolchain"];
      if (!isRecord(tc)) {
        throw new ToolchainError("NS9101", "toolchain must be an object");
      }
      if (Object.prototype.hasOwnProperty.call(tc, "c")) {
        raw = tc["c"];
      }
    }
  }
  if (raw === undefined) raw = "auto";
  if (typeof raw !== "string") {
    throw new ToolchainError("NS9101", "toolchain.c must be a string");
  }
  const request = raw.trim();
  if (request === "") {
    throw new ToolchainError("NS9101", "toolchain.c must not be empty");
  }
  if (request === "auto") {
    return { request: "auto" };
  }

  if (/\s/.test(request)) {
    throw new ToolchainError("NS9101", "toolchain.c must not contain whitespace");
  }
  const parts = request.split("@");
  if (parts.length > 2) {
    throw new ToolchainError("NS9101", "toolchain.c must not contain multiple '@'");
  }
  const family = parts[0] ?? "";
  const version = parts.length === 2 ? (parts[1] ?? "") : "";
  if (!FAMILIES.has(family)) {
    throw new ToolchainError("NS9101", `unknown toolchain family '${family}'`);
  }
  if (parts.length === 2 && version === "") {
    throw new ToolchainError("NS9101", "toolchain.c version must not be empty");
  }
  return { request };
}

function familyFromBinary(name: string): string {
  const base = name.replace(/\.exe$/i, "").toLowerCase();
  if (base === "clang") return "clang";
  if (base === "gcc" || base === "cc") return "gcc";
  if (base === "zig") return "zig";
  return base;
}

function findOnPath(name: string, probe: ToolchainProbe): string | null {
  const result = probe.run(name, ["--version"]);
  return result.status === 0 ? name : null;
}

function versionOf(probe: ToolchainProbe, bin: string): string | null {
  const result = probe.run(bin, ["--version"]);
  if (result.status !== 0) return null;
  const line = result.stdout.trim().split(/\r?\n/)[0]?.trim() ?? "";
  return line === "" ? null : line;
}

function record(options: ResolveOptions, rung: ToolchainRung, detail: string): void {
  options.onAttempt?.({ rung, detail });
}

interface CacheResult {
  readonly resolution: ToolchainResolution | null;
  readonly reason: string;
  readonly canFallBack: boolean;
}

function lookupCache(projectRoot: string, id: string, probe: ToolchainProbe): CacheResult {
  const at = id.indexOf("@");
  const family = at >= 0 ? id.slice(0, at) : id;
  const versioned = at >= 0;
  const cacheDir = `${projectRoot}/.tools/toolchain/${id}`;
  const markerPath = `${cacheDir}/toolchain.json`;
  if (!probe.exists(markerPath)) {
    return {
      resolution: null,
      reason: `no cache marker at ${markerPath}`,
      canFallBack: !versioned,
    };
  }
  let marker: unknown;
  try {
    marker = JSON.parse(probe.readFileSync(markerPath));
  } catch {
    return {
      resolution: null,
      reason: `invalid cache marker at ${markerPath}`,
      canFallBack: false,
    };
  }
  if (!isRecord(marker)) {
    return {
      resolution: null,
      reason: `invalid cache marker at ${markerPath}`,
      canFallBack: false,
    };
  }
  if (marker["id"] !== id) {
    return {
      resolution: null,
      reason: `cache marker id mismatch: expected ${id}, got ${String(marker["id"])}`,
      canFallBack: false,
    };
  }
  if (marker["family"] !== family) {
    return {
      resolution: null,
      reason: `cache marker family mismatch: expected ${family}, got ${String(marker["family"])}`,
      canFallBack: false,
    };
  }
  const markerVersion = marker["version"];
  if (typeof markerVersion !== "string") {
    return {
      resolution: null,
      reason: `cache marker has no version at ${markerPath}`,
      canFallBack: false,
    };
  }
  if (versioned && markerVersion !== id.slice(at + 1)) {
    return {
      resolution: null,
      reason: `cache marker version mismatch: expected ${id.slice(at + 1)}, got ${markerVersion}`,
      canFallBack: false,
    };
  }
  const candidates = [
    `${cacheDir}/bin/${family}`,
    `${cacheDir}/bin/${family}.exe`,
    `${cacheDir}/${family}`,
    `${cacheDir}/${family}.exe`,
  ];
  const bin = candidates.find((candidate) => probe.exists(candidate));
  if (bin === undefined) {
    return {
      resolution: null,
      reason: `cache marker present but binary missing for ${family} in ${cacheDir}`,
      canFallBack: false,
    };
  }
  return {
    resolution: { rung: "cache", id, family, path: bin, version: markerVersion },
    reason: "",
    canFallBack: false,
  };
}

export function resolveToolchain(options: ResolveOptions): ToolchainResolution {
  const { projectRoot, policy, env, probe } = options;

  const astraCc = env.ASTRA_CC?.trim();
  if (astraCc) {
    record(options, "env", `ASTRA_CC=${astraCc}`);
    const exists = probe.exists(astraCc);
    const path = exists ? astraCc : findOnPath(astraCc, probe);
    if (path === null) {
      throw new ToolchainError(
        "NS9103",
        `ASTRA_CC does not exist and is not on PATH: ${astraCc}`,
      );
    }
    const version = versionOf(probe, path);
    if (version === null) {
      throw new ToolchainError("NS9103", `ASTRA_CC does not answer --version: ${astraCc}`);
    }
    const family = familyFromBinary(path.split(/[/\\]/).pop() ?? path);
    return { rung: "env", id: family, family, path, version };
  }

  if (policy.request !== "auto") {
    const id = policy.request;
    const cache = lookupCache(projectRoot, id, probe);
    if (cache.resolution !== null) {
      record(options, "cache", `resolved ${cache.resolution.path}`);
      return cache.resolution;
    }
    record(options, "cache", cache.reason);
    if (!cache.canFallBack) {
      throw new ToolchainError("NS9102", cache.reason);
    }
    const family = id;
    const pathBin = findOnPath(family, probe);
    if (pathBin !== null) {
      const version = versionOf(probe, pathBin);
      if (version !== null) {
        record(options, "path", `resolved ${pathBin}`);
        return { rung: "path", id: family, family, path: pathBin, version };
      }
      record(options, "path", `${family}: --version did not answer`);
    } else {
      record(options, "path", `${family}: not on PATH`);
    }
    throw new ToolchainError("NS9102", `no cached or PATH toolchain for ${family}`);
  }

  const candidates = ["clang", "gcc", "cc"];
  for (const candidate of candidates) {
    const bin = findOnPath(candidate, probe);
    if (bin === null) {
      record(options, "path", `${candidate}: not on PATH`);
      continue;
    }
    const hosted = probe.run(bin, ["-std=c17", "-fsyntax-only", "-x", "c", "-"]);
    const version = versionOf(probe, bin);
    record(
      options,
      "path",
      `${candidate}: hosted probe ${hosted.status === 0 ? "passed" : "failed"}, --version ${
        version === null ? "failed" : "ok"
      }`,
    );
    if (hosted.status === 0 && version !== null) {
      const family = familyFromBinary(candidate);
      return { rung: "path", id: family, family, path: bin, version };
    }
  }
  for (const candidate of candidates) {
    const bin = findOnPath(candidate, probe);
    if (bin === null) continue;
    const version = versionOf(probe, bin);
    if (version === null) continue;
    record(options, "path", `${candidate}: fallback --version responder`);
    const family = familyFromBinary(candidate);
    return { rung: "path", id: family, family, path: bin, version };
  }
  throw new ToolchainError("NS9102", "no toolchain found on PATH");
}

export interface DoctorResult {
  readonly attempts: readonly ToolchainAttempt[];
  readonly winner: ToolchainResolution | null;
  readonly error: string | null;
}

export function doctor(
  projectRoot: string,
  policy: ToolchainPolicy,
  env: { readonly ASTRA_CC?: string },
  probe: ToolchainProbe,
): DoctorResult {
  const attempts: ToolchainAttempt[] = [];
  let winner: ToolchainResolution | null = null;
  let error: string | null = null;
  try {
    winner = resolveToolchain({
      projectRoot,
      policy,
      env,
      probe,
      onAttempt: (attempt) => attempts.push(attempt),
    });
  } catch (e) {
    error = e instanceof ToolchainError ? `${e.code}: ${e.message}` : e instanceof Error ? e.message : String(e);
  }
  return { attempts, winner, error };
}

export interface PinnedArtifact {
  readonly id: "zig@0.13.0";
  readonly version: "0.13.0";
  readonly family: "zig";
  readonly platform: "linux" | "darwin" | "win32";
  readonly arch: "x64" | "arm64";
  readonly url: string;
  readonly sha256: string;
  readonly size: number;
  readonly format: "tar.xz" | "zip";
}

export const PINNED_ARTIFACTS: readonly PinnedArtifact[] = [
  {
    id: "zig@0.13.0",
    version: "0.13.0",
    family: "zig",
    platform: "linux",
    arch: "x64",
    url: "https://ziglang.org/download/0.13.0/zig-linux-x86_64-0.13.0.tar.xz",
    sha256: "d45312e61ebcc48032b77bc4cf7fd6915c11fa16e4aad116b66c9468211230ea",
    size: 47082308,
    format: "tar.xz",
  },
  {
    id: "zig@0.13.0",
    version: "0.13.0",
    family: "zig",
    platform: "linux",
    arch: "arm64",
    url: "https://ziglang.org/download/0.13.0/zig-linux-aarch64-0.13.0.tar.xz",
    sha256: "041ac42323837eb5624068acd8b00cd5777dac4cf91179e8dad7a7e90dd0c556",
    size: 43090688,
    format: "tar.xz",
  },
  {
    id: "zig@0.13.0",
    version: "0.13.0",
    family: "zig",
    platform: "darwin",
    arch: "x64",
    url: "https://ziglang.org/download/0.13.0/zig-macos-x86_64-0.13.0.tar.xz",
    sha256: "8b06ed1091b2269b700b3b07f8e3be3b833000841bae5aa6a09b1a8b4773effd",
    size: 48857012,
    format: "tar.xz",
  },
  {
    id: "zig@0.13.0",
    version: "0.13.0",
    family: "zig",
    platform: "darwin",
    arch: "arm64",
    url: "https://ziglang.org/download/0.13.0/zig-macos-aarch64-0.13.0.tar.xz",
    sha256: "46fae219656545dfaf4dce12fb4e8685cec5b51d721beee9389ab4194d43394c",
    size: 44892040,
    format: "tar.xz",
  },
  {
    id: "zig@0.13.0",
    version: "0.13.0",
    family: "zig",
    platform: "win32",
    arch: "x64",
    url: "https://ziglang.org/download/0.13.0/zig-windows-x86_64-0.13.0.zip",
    sha256: "d859994725ef9402381e557c60bb57497215682e355204d754ee3df75ee3c158",
    size: 79163968,
    format: "zip",
  },
  {
    id: "zig@0.13.0",
    version: "0.13.0",
    family: "zig",
    platform: "win32",
    arch: "arm64",
    url: "https://ziglang.org/download/0.13.0/zig-windows-aarch64-0.13.0.zip",
    sha256: "95ff88427af7ba2b4f312f45d2377ce7a033e5e3c620c8caaa396a9aba20efda",
    size: 75119033,
    format: "zip",
  },
];

function normalizeArch(arch: string): string {
  if (arch === "x86_64") return "x64";
  if (arch === "aarch64") return "arm64";
  return arch;
}

export function pinnedArtifact(
  id: string,
  platform?: NodeJS.Platform,
  arch?: string,
  artifacts: readonly PinnedArtifact[] = PINNED_ARTIFACTS,
): PinnedArtifact | null {
  const hostPlatform = platform ?? process.platform;
  const hostArch = normalizeArch(arch ?? process.arch);
  return (
    artifacts.find(
      (artifact) =>
        artifact.id === id &&
        artifact.platform === hostPlatform &&
        artifact.arch === hostArch,
    ) ?? null
  );
}

export interface NativeToolchainInvocation {
  readonly bin: string;
  readonly args: readonly string[];
}

export function compilerInvocation(
  resolution: ToolchainResolution,
): NativeToolchainInvocation {
  if (resolution.family === "zig") {
    return { bin: resolution.path, args: ["cc"] };
  }
  return { bin: resolution.path, args: [] };
}

export function archiverInvocation(
  resolution: ToolchainResolution,
): NativeToolchainInvocation {
  if (resolution.family === "zig") {
    return { bin: resolution.path, args: ["ar"] };
  }
  return { bin: resolution.path, args: [] };
}

export interface AsyncResolveOptions extends ResolveOptions {
  readonly allowDownload: boolean;
  readonly fetchImpl?: typeof fetch;
  readonly platform?: NodeJS.Platform;
  readonly arch?: string;

  readonly artifacts?: readonly PinnedArtifact[];
}

export interface InstallOptions {
  readonly fetchImpl?: typeof fetch;
  readonly platform?: NodeJS.Platform;
  readonly arch?: string;
  readonly artifacts?: readonly PinnedArtifact[];
}

function cacheDirFor(projectRoot: string, id: string): string {
  return join(projectRoot, ".tools", "toolchain", id);
}

function manualInstructions(
  artifact: PinnedArtifact | null,
  id: string,
  cacheDir: string,
): string {
  if (artifact === null) {
    const at = id.indexOf("@");
    const family = at < 0 ? id : id.slice(0, at);
    const version = at < 0 ? "unknown" : id.slice(at + 1);
    return `Manual install: extract the ${family} toolchain into ${cacheDir} and write ${join(cacheDir, "toolchain.json")} with {"id":"${id}","family":"${family}","version":"${version}"}. Use --no-download to skip network access and install manually.`;
  }
  return `Manual install: download ${artifact.url}, verify sha256 ${artifact.sha256}, extract the ${artifact.family} binary to ${join(cacheDir, artifact.family)} (or ${join(cacheDir, "bin", artifact.family)}), and write ${join(cacheDir, "toolchain.json")} with {"id":"${artifact.id}","family":"${artifact.family}","version":"${artifact.version}"}. Use --no-download to skip network access and install manually.`;
}

function makeProbe(): ToolchainProbe {
  return {
    exists: (path: string) => existsSync(path),
    run: (command: string, args: readonly string[]) => {
      const result = spawnSync(command, [...args], {
        encoding: "utf8",
        windowsHide: true,
      });
      return {
        status: result.status,
        stdout: result.stdout ?? "",
        stderr: result.stderr ?? result.error?.message ?? "",
      };
    },
    readFileSync: (path: string) => readFileSync(path, "utf8"),
    readdir: (path: string) => readdirSync(path),
  };
}

function systemTar(): string {
  if (process.platform === "win32") {
    const systemTar = `${process.env["SystemRoot"] ?? "C:\\Windows"}\\System32\\tar.exe`;
    if (existsSync(systemTar)) return systemTar;
  }
  return "tar";
}

export async function installPinnedToolchain(
  projectRoot: string,
  id: string,
  options?: InstallOptions,
): Promise<ToolchainResolution> {
  const cacheDir = cacheDirFor(projectRoot, id);
  const artifact = pinnedArtifact(id, options?.platform, options?.arch, options?.artifacts);
  if (artifact === null) {
    throw new ToolchainError(
      "NS9102",
      `No pinned artifact for ${id} on this host. ${manualInstructions(null, id, cacheDir)}`,
    );
  }

  const probe = makeProbe();
  const cached = lookupCache(projectRoot, id, probe);
  if (cached.resolution !== null) return cached.resolution;
  if (probe.exists(join(cacheDir, "toolchain.json"))) {

    throw new ToolchainError(
      "NS9102",
      `${cached.reason}; ${manualInstructions(artifact, id, cacheDir)}`,
    );
  }

  const fetchImpl = options?.fetchImpl ?? fetch;
  let response: Response;
  try {
    response = await fetchImpl(artifact.url, {
      headers: {
        "User-Agent": "neo-astra-toolchain/0.1.0",
        Accept: "application/octet-stream",
      },
    });
  } catch (error) {
    throw new ToolchainError(
      "NS9102",
      `download failed for ${artifact.url}: ${
        error instanceof Error ? error.message : String(error)
      }; ${manualInstructions(artifact, id, cacheDir)}`,
    );
  }
  if (!response.ok) {
    throw new ToolchainError(
      "NS9102",
      `download failed (HTTP ${response.status}) for ${artifact.url}; ${manualInstructions(artifact, id, cacheDir)}`,
    );
  }
  const bytes = Buffer.from(await response.arrayBuffer());
  if (bytes.length !== artifact.size) {
    throw new ToolchainError(
      "NS9102",
      `archive size mismatch: expected ${artifact.size} bytes, received ${bytes.length}; ${manualInstructions(artifact, id, cacheDir)}`,
    );
  }
  const digest = createHash("sha256").update(bytes).digest("hex");
  if (digest !== artifact.sha256) {
    throw new ToolchainError(
      "NS9102",
      `archive sha256 mismatch: expected ${artifact.sha256}, received ${digest}; ${manualInstructions(artifact, id, cacheDir)}`,
    );
  }

  const toolsDir = join(projectRoot, ".tools", "toolchain");
  mkdirSync(toolsDir, { recursive: true });
  const suffix = `${Date.now()}-${Math.random().toString(36).slice(2)}`;
  const staging = join(toolsDir, `.staging-${suffix}`);
  const extracted = join(staging, "extracted");
  const installDir = join(staging, "install");
  const backupDir = join(toolsDir, `.backup-${suffix}`);
  let movedOldCache = false;
  try {
    mkdirSync(extracted, { recursive: true });
    mkdirSync(installDir, { recursive: true });
    const archivePath = join(staging, "archive");
    writeFileSync(archivePath, bytes);

    const extraction = spawnSync(
      systemTar(),
      ["-xf", archivePath, "-C", extracted],
      { windowsHide: true, encoding: "utf8" },
    );
    if (extraction.status !== 0) {
      throw new ToolchainError(
        "NS9102",
        `archive extraction failed: ${
          extraction.stderr || extraction.error?.message || "unknown error"
        }; ${manualInstructions(artifact, id, cacheDir)}`,
      );
    }

    const entries = readdirSync(extracted);
    if (entries.length === 0) {
      throw new ToolchainError(
        "NS9102",
        `the downloaded archive was empty; ${manualInstructions(artifact, id, cacheDir)}`,
      );
    }
    const onlyEntry = entries.length === 1 ? join(extracted, entries[0] as string) : null;
    const sourceRoot =
      onlyEntry !== null && statSync(onlyEntry).isDirectory() ? onlyEntry : extracted;
    for (const entry of readdirSync(sourceRoot)) {
      renameSync(join(sourceRoot, entry), join(installDir, entry));
    }

    const binaryName =
      artifact.platform === "win32" ? `${artifact.family}.exe` : artifact.family;
    const binaryCandidates = [
      join(installDir, binaryName),
      join(installDir, "bin", binaryName),
    ];
    if (!binaryCandidates.some((candidate) => existsSync(candidate))) {
      throw new ToolchainError(
        "NS9102",
        `the archive did not contain ${binaryName}; ${manualInstructions(artifact, id, cacheDir)}`,
      );
    }

    writeFileSync(
      join(installDir, "toolchain.json"),
      `${JSON.stringify(
        {
          id,
          family: artifact.family,
          version: artifact.version,
          url: artifact.url,
          sha256: artifact.sha256,
        },
        null,
        2,
      )}\n`,
      "utf8",
    );

    if (existsSync(cacheDir)) {
      renameSync(cacheDir, backupDir);
      movedOldCache = true;
    }
    try {
      renameSync(installDir, cacheDir);
    } catch (error) {

      const concurrent = lookupCache(projectRoot, id, probe);
      if (concurrent.resolution !== null) {
        rmSync(staging, { recursive: true, force: true });
        if (movedOldCache) rmSync(backupDir, { recursive: true, force: true });
        return concurrent.resolution;
      }
      throw error;
    }
    if (movedOldCache) rmSync(backupDir, { recursive: true, force: true });
    rmSync(staging, { recursive: true, force: true });

    const installed = lookupCache(projectRoot, id, probe);
    if (installed.resolution === null) {
      throw new ToolchainError(
        "NS9102",
        `installed cache could not be verified: ${installed.reason}`,
      );
    }
    return installed.resolution;
  } catch (error) {
    try {
      rmSync(staging, { recursive: true, force: true });
    } catch {

    }
    if (movedOldCache && existsSync(backupDir) && !existsSync(cacheDir)) {
      try {
        renameSync(backupDir, cacheDir);
      } catch {

      }
    }
    if (error instanceof ToolchainError) throw error;
    throw new ToolchainError(
      "NS9102",
      `install failed: ${
        error instanceof Error ? error.message : String(error)
      }; ${manualInstructions(artifact, id, cacheDir)}`,
    );
  }
}

export async function resolveToolchainAsync(
  options: AsyncResolveOptions,
): Promise<ToolchainResolution> {
  const {
    projectRoot,
    policy,
    env,
    probe,
    allowDownload,
    fetchImpl,
    platform,
    arch,
    artifacts,
  } = options;
  const effectiveProbe = probe ?? makeProbe();

  if (env.ASTRA_CC?.trim()) {
    return resolveToolchain({ projectRoot, policy, env, probe: effectiveProbe });
  }
  if (policy.request === "auto") {
    return resolveToolchain({ projectRoot, policy, env, probe: effectiveProbe });
  }

  const id = policy.request;
  if (!id.includes("@")) {
    return resolveToolchain({ projectRoot, policy, env, probe: effectiveProbe });
  }

  const cached = lookupCache(projectRoot, id, effectiveProbe);
  if (cached.resolution !== null) return cached.resolution;

  const cacheDir = cacheDirFor(projectRoot, id);
  const artifact = pinnedArtifact(id, platform, arch, artifacts);
  if (effectiveProbe.exists(join(cacheDir, "toolchain.json"))) {
    throw new ToolchainError(
      "NS9102",
      `${cached.reason}; ${manualInstructions(artifact, id, cacheDir)}`,
    );
  }
  if (!allowDownload) {
    throw new ToolchainError(
      "NS9102",
      `download disabled (--no-download); ${manualInstructions(artifact, id, cacheDir)}`,
    );
  }
  if (artifact === null) {
    throw new ToolchainError(
      "NS9102",
      `no pinned artifact for ${id} on this host; ${manualInstructions(null, id, cacheDir)}`,
    );
  }
  return installPinnedToolchain(projectRoot, id, {
    ...(fetchImpl === undefined ? {} : { fetchImpl }),
    ...(platform === undefined ? {} : { platform }),
    ...(arch === undefined ? {} : { arch }),
    ...(artifacts === undefined ? {} : { artifacts }),
  });
}

export async function doctorAsync(
  projectRoot: string,
  policy: ToolchainPolicy,
  env: { readonly ASTRA_CC?: string },
  probe: ToolchainProbe,
  options?: {
    readonly allowDownload?: boolean;
    readonly fetchImpl?: typeof fetch;
    readonly platform?: NodeJS.Platform;
    readonly arch?: string;
    readonly artifacts?: readonly PinnedArtifact[];
  },
): Promise<DoctorResult> {
  const base = doctor(projectRoot, policy, env, probe);
  if (base.winner !== null) return base;
  if (!options?.allowDownload) {
    if (
      policy.request !== "auto" &&
      policy.request.includes("@") &&
      !env.ASTRA_CC?.trim()
    ) {
      return {
        attempts: [
          ...base.attempts,
          { rung: "cache", detail: "download disabled (--no-download)" },
        ],
        winner: null,
        error: base.error,
      };
    }
    return base;
  }
  if (env.ASTRA_CC?.trim()) return base;
  const id = policy.request;
  if (id === "auto" || !id.includes("@")) return base;

  try {
    const installed = await installPinnedToolchain(projectRoot, id, options);
    return {
      attempts: [...base.attempts, { rung: "cache", detail: `downloaded ${id}` }],
      winner: installed,
      error: null,
    };
  } catch (error) {
    const message =
      error instanceof ToolchainError
        ? `${error.code}: ${error.message}`
        : error instanceof Error
          ? error.message
          : String(error);
    return {
      attempts: [
        ...base.attempts,
        { rung: "cache", detail: `download failed: ${message}` },
      ],
      winner: null,
      error: message,
    };
  }
}
