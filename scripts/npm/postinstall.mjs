#!/usr/bin/env node

import { spawnSync } from "node:child_process";
import {
  chmodSync,
  copyFileSync,
  existsSync,
  mkdirSync,
  mkdtempSync,
  readFileSync,
  rmSync,
} from "node:fs";
import { createRequire } from "node:module";
import { tmpdir } from "node:os";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

const here = dirname(fileURLToPath(import.meta.url));
const require = createRequire(import.meta.url);
const manifest = JSON.parse(readFileSync(join(here, "package.json"), "utf8"));
const osName = { win32: "windows", linux: "linux", darwin: "darwin" }[
  process.platform
];
const arch = process.arch;
const target = osName === undefined ? undefined : `neo-astra-${osName}-${arch}`;
const supported = new Set([
  "neo-astra-windows-x64",
  "neo-astra-linux-x64",
  "neo-astra-darwin-arm64",
]);
const executable = process.platform === "win32" ? ".exe" : "";

function fail(message) {
  process.stderr.write(`[neo-astra] ${message}\n`);
  process.exit(1);
}

function copyRequired(source, targetPath) {
  if (!existsSync(source)) {
    throw new Error(`platform package is missing ${source}`);
  }
  mkdirSync(dirname(targetPath), { recursive: true });
  copyFileSync(source, targetPath);
}

function copyToolchain(source) {
  const bin = join(here, "bin");
  const typescript = join(here, "typescript");
  rmSync(bin, { recursive: true, force: true });
  rmSync(typescript, { recursive: true, force: true });
  mkdirSync(bin, { recursive: true });
  mkdirSync(typescript, { recursive: true });

  const sourceBin = join(source, "bin");
  const astra = join(bin, "astra.exe");
  copyRequired(join(sourceBin, `astra${executable}`), astra);
  copyRequired(
    join(sourceBin, `astra-compiler${executable}`),
    join(bin, `astra-compiler${executable}`),
  );
  copyRequired(
    join(sourceBin, `nova${executable}`),
    join(bin, "nova.exe"),
  );
  copyRequired(
    join(sourceBin, `nopm${executable}`),
    join(bin, "nopm.exe"),
  );
  copyRequired(
    join(source, "typescript", `tsc${executable}`),
    join(typescript, `tsc${executable}`),
  );
  copyRequired(
    join(source, "typescript", "package.json"),
    join(typescript, "package.json"),
  );

  if (process.platform !== "win32") {
    for (const file of [
      "astra.exe",
      "nopm.exe",
      "astra-compiler",
      "nova.exe",
    ]) {
      chmodSync(join(bin, file), 0o755);
    }
    chmodSync(join(typescript, "tsc"), 0o755);
  }
}

function findPlatformPackage() {
  try {
    return dirname(require.resolve(`${target}/package.json`));
  } catch {
    return undefined;
  }
}

function installPlatformPackage() {
  const version = manifest.optionalDependencies?.[target] ?? manifest.version;
  const temp = mkdtempSync(join(tmpdir(), "neo-astra-install-"));
  try {
    const result = spawnSync(
      process.platform === "win32" ? "npm.cmd" : "npm",
      [
        "install",
        "--ignore-scripts",
        "--no-save",
        "--loglevel=error",
        "--prefix",
        temp,
        `${target}@${version}`,
      ],
      {
        stdio: "inherit",
        shell: process.platform === "win32",
        windowsHide: true,
      },
    );
    if (result.status !== 0) {
      throw new Error(
        `could not install ${target}@${version}; reinstall without --ignore-scripts or use the GitHub archive`,
      );
    }
    const source = join(temp, "node_modules", target);
    if (!existsSync(source)) {
      throw new Error(`npm did not install ${target}@${version}`);
    }
    copyToolchain(source);
  } finally {
    rmSync(temp, { recursive: true, force: true });
  }
}

function verify() {
  const astra = spawnSync(join(here, "bin", "astra.exe"), ["--version"], {
    encoding: "utf8",
    stdio: "ignore",
    windowsHide: true,
  });
  if (astra.status !== 0) {
    throw new Error("the installed astra binary did not start");
  }
  const nopm = spawnSync(join(here, "bin", "nopm.exe"), ["--version"], {
    encoding: "utf8",
    stdio: "ignore",
    windowsHide: true,
  });
  if (nopm.status !== 0) {
    throw new Error("the installed nopm binary did not start");
  }
}

if (target === undefined || !supported.has(target)) {
  fail(
    `no npm build is published for ${process.platform}/${process.arch}. ` +
      "Supported npm hosts: Windows x64, Linux x64, and macOS arm64. " +
      "Use the GitHub archive or build from a source checkout on this host.",
  );
}

const installed = findPlatformPackage();
if (installed !== undefined) {
  try {
    copyToolchain(installed);
    verify();
    process.exit(0);
  } catch {

  }
}

try {
  installPlatformPackage();
  verify();
} catch (error) {
  fail(error instanceof Error ? error.message : String(error));
}
