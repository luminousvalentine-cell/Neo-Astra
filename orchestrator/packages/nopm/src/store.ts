import { spawnSync } from "node:child_process";
import { createHash } from "node:crypto";
import {
  cpSync,
  existsSync,
  mkdirSync,
  readdirSync,
  renameSync,
  rmSync,
  statSync,
  writeFileSync,
} from "node:fs";
import { tmpdir } from "node:os";
import { join, relative, resolve } from "node:path";

import { NopmError, type Lockfile, type LockfilePackage } from "./index.js";

export interface StoreIndexEntry {
  readonly id: string;
  readonly source: string;
  readonly root: string;
  readonly integrity: string | null;
  readonly targets: readonly string[];
}

export interface StoreIndex {
  readonly version: 1;
  readonly generatedBy: string;
  readonly packages: readonly StoreIndexEntry[];
}

export interface InstallOptions {
  readonly fetchImpl?: typeof fetch;
  readonly generatedBy: string;
}

export async function installLockedPackages(
  projectRoot: string,
  lockfile: Lockfile,
  options: InstallOptions,
): Promise<StoreIndex> {
  const storeDirectory = join(projectRoot, ".astra", "store");
  mkdirSync(storeDirectory, { recursive: true });
  const entries: StoreIndexEntry[] = [];

  for (const entry of [...lockfile.packages].sort((left, right) =>
    left.id.localeCompare(right.id),
  )) {
    entries.push(await installPackage(projectRoot, storeDirectory, entry, options));
  }

  const index: StoreIndex = Object.freeze({
    version: 1,
    generatedBy: options.generatedBy,
    packages: Object.freeze(entries),
  });
  writeStoreIndex(projectRoot, index);
  return index;
}

async function installPackage(
  projectRoot: string,
  storeDirectory: string,
  entry: LockfilePackage,
  options: InstallOptions,
): Promise<StoreIndexEntry> {
  if (entry.source === "path") {
    if (entry.resolved === null) {
      throw new NopmError("NP0011", `Path package '${entry.id}' has no resolved path.`, entry.id);
    }
    const root = resolve(projectRoot, entry.resolved);
    if (!existsSync(root)) {
      throw new NopmError("NP0011", `Path package '${entry.id}' is missing.`, root);
    }
    return Object.freeze({
      id: entry.id,
      source: entry.source,
      root: normalizeRelative(projectRoot, root),
      integrity: entry.integrity,
      targets: entry.targets,
    });
  }

  const key = storeKey(entry);
  const storePath = join(storeDirectory, key);

  if (!existsSync(storePath)) {
    if (entry.source === "git") {
      installGitCache(projectRoot, entry, storePath);
    } else {
      await downloadAndExtract(entry, storePath, options.fetchImpl ?? fetch);
    }
  }

  return Object.freeze({
    id: entry.id,
    source: entry.source,
    root: normalizeRelative(projectRoot, storePath),
    integrity: entry.integrity,
    targets: entry.targets,
  });
}

function installGitCache(projectRoot: string, entry: LockfilePackage, storePath: string): void {
  if (entry.commit === null) {
    throw new NopmError("NP0011", `Git package '${entry.id}' has no pinned commit.`, entry.id);
  }
  const cachePath = join(projectRoot, ".astra", "cache", "git", entry.commit);
  if (!existsSync(cachePath)) {
    throw new NopmError(
      "NP0011",
      `Git package '${entry.id}' is not in the local cache. Run nopm lock again.`,
      cachePath,
    );
  }
  copyDirectory(cachePath, storePath);
}

async function downloadAndExtract(
  entry: LockfilePackage,
  storePath: string,
  fetchImpl: typeof fetch,
): Promise<void> {
  if (entry.resolved === null || entry.integrity === null) {
    throw new NopmError(
      "NP0011",
      `Package '${entry.id}' has no resolved URL or integrity hash.`,
      entry.id,
    );
  }

  let response: Response;
  try {
    response = await fetchImpl(entry.resolved, {
      headers: { "User-Agent": "neo-astra-nopm/0.1.0" },
    });
  } catch (error) {
    throw new NopmError(
      "NP0011",
      `Unable to download '${entry.id}': ${error instanceof Error ? error.message : String(error)}`,
      entry.resolved,
      { cause: error },
    );
  }
  if (!response.ok) {
    throw new NopmError("NP0011", `Unable to download '${entry.id}': HTTP ${response.status}.`, entry.resolved);
  }

  const bytes = Buffer.from(await response.arrayBuffer());
  verifyIntegrityBytes(bytes, entry.integrity);

  const working = join(tmpdir(), `nopm-install-${process.pid}-${Date.now()}`);
  const archive = join(working, "package.tgz");
  const extracted = join(working, "extracted");
  mkdirSync(extracted, { recursive: true });
  try {
    writeFileSync(archive, bytes);
    let result = spawnSync("tar", ["-xf", archive, "-C", extracted], {
      encoding: "utf8",
      windowsHide: true,
    });
    if (result.status !== 0) {
      result = spawnSync("unzip", ["-q", archive, "-d", extracted], {
        encoding: "utf8",
        windowsHide: true,
      });
    }
    if (result.status !== 0) {
      throw new NopmError(
        "NP0011",
        `Unable to extract '${entry.id}': ${(result.stderr ?? result.error?.message ?? "").trim()}`,
        entry.resolved,
      );
    }
    const root = packageRoot(extracted);
    copyDirectory(root, storePath);
  } finally {
    rmSync(working, { recursive: true, force: true });
  }
}

function packageRoot(directory: string): string {
  const entries = readdirSync(directory).filter((name) => !name.startsWith("."));
  if (entries.length === 1) {
    const only = join(directory, entries[0] ?? "");
    if (existsSync(only) && statSync(only).isDirectory()) {
      return only;
    }
  }
  return directory;
}

function copyDirectory(source: string, destination: string): void {
  const temporary = `${destination}.tmp-${process.pid}-${Date.now()}`;
  try {
    mkdirSync(temporary, { recursive: true });
    cpSync(source, temporary, { recursive: true });
    renameSync(temporary, destination);
  } catch (error) {
    rmSync(temporary, { recursive: true, force: true });
    throw new NopmError(
      "NP0011",
      `Unable to copy package content to the store: ${error instanceof Error ? error.message : String(error)}`,
      destination,
      { cause: error },
    );
  }
}

function verifyIntegrityBytes(bytes: Buffer, integrity: string): void {
  if (integrity.startsWith("git:")) {
    return;
  }

  const sri = /^(sha256|sha384|sha512)-(.+)$/.exec(integrity);
  if (sri !== null) {
    const algorithm = sri[1] as "sha256" | "sha384" | "sha512";
    const expected = sri[2] ?? "";
    const actual = createHash(algorithm).update(bytes).digest("base64");
    if (actual !== expected) {
      throw integrityMismatch(integrity);
    }
    return;
  }

  const hex = /^(sha256|sha384|sha512):(.+)$/.exec(integrity);
  if (hex !== null) {
    const algorithm = hex[1] as "sha256" | "sha384" | "sha512";
    const expected = (hex[2] ?? "").toLowerCase();
    const actual = createHash(algorithm).update(bytes).digest("hex");
    if (actual !== expected) {
      throw integrityMismatch(integrity);
    }
    return;
  }

  throw new NopmError("NP0011", `Unsupported integrity format '${integrity}'.`, integrity);
}

function integrityMismatch(integrity: string): NopmError {
  return new NopmError("NP0011", `Integrity check failed for '${integrity}'.`, integrity);
}

function storeKey(entry: LockfilePackage): string {
  const identity = `${entry.id}\0${entry.integrity ?? ""}\0${entry.commit ?? ""}`;
  return createHash("sha256").update(identity, "utf8").digest("hex");
}

function writeStoreIndex(projectRoot: string, index: StoreIndex): void {
  const path = join(projectRoot, ".astra", "store", "index.json");
  const temporary = `${path}.tmp-${process.pid}-${Date.now()}`;
  try {
    writeFileSync(temporary, `${JSON.stringify(index, null, 2)}\n`, "utf8");
    renameSync(temporary, path);
  } catch (error) {
    rmSync(temporary, { force: true });
    throw new NopmError("NP0011", "Unable to write the package store index.", path, {
      cause: error,
    });
  }
}

function normalizeRelative(projectRoot: string, path: string): string {
  return relative(projectRoot, path).split("\\").join("/");
}
