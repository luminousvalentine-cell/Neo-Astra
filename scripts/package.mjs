#!/usr/bin/env node

import { spawnSync } from "node:child_process";
import { createHash } from "node:crypto";
import {
  chmodSync,
  copyFileSync,
  existsSync,
  mkdirSync,
  mkdtempSync,
  readdirSync,
  readFileSync,
  rmSync,
  statSync,
  writeFileSync,
} from "node:fs";
import { tmpdir } from "node:os";
import { delimiter, dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

const root = fileURLToPath(new URL("..", import.meta.url));
const skipBuild = process.argv.includes("--skip-build");
const version = JSON.parse(readFileSync(join(root, "package.json"), "utf8")).version;
const platform = process.platform;
const arch = process.arch;
const npm = platform === "win32" ? "npm.cmd" : "npm";
const executable = platform === "win32" ? ".exe" : "";
const releaseName = `neo-astra-${version}-${platform}-${arch}`;
const outRoot = join(root, "dist", releaseName);
const binDir = join(outRoot, "bin");

function fail(message) {
  process.stderr.write(`[package] ${message}\n`);
  process.exit(1);
}

function run(command, args, options = {}) {
  process.stdout.write(`[package] ${command} ${args.join(" ")}\n`);
  const result = spawnSync(command, args, {
    cwd: root,
    stdio: "inherit",
    shell: platform === "win32" && command.endsWith(".cmd"),
    ...options,
  });
  if (result.error) {
    fail(result.error.message);
  }
  if (result.status !== 0) {
    fail(`${command} exited with status ${String(result.status)}`);
  }
}

function smokeRun(label, command, args, options = {}) {
  const result = spawnSync(command, args, {
    encoding: "utf8",
    windowsHide: true,
    ...options,
  });
  if (result.error) {
    fail(`${label}: ${result.error.message}`);
  }
  if (result.status !== 0) {
    const output = [result.stdout, result.stderr]
      .filter((value) => value !== undefined && value !== "")
      .join("\n")
      .trim();
    fail(
      `${label} failed (status ${String(result.status)})${output ? `:\n${output}` : ""}`,
    );
  }
  return result;
}

function writeSmokeProject(directory, source, targets, defaultTarget) {
  mkdirSync(join(directory, "src"), { recursive: true });
  writeFileSync(
    join(directory, "core.json"),
    `${JSON.stringify(
      {
        package: {
          name: "package-smoke",
          version: "0.0.0",
          edition: "2026",
          license: "Apache-2.0",
        },
      },
      null,
      2,
    )}\n`,
    "utf8",
  );
  writeFileSync(
    join(directory, "build.json"),
    `${JSON.stringify(
      {
        build: { defaultTarget, defaultMode: "core" },
        nova: { entry: "src/main.ns" },
        targets,
      },
      null,
      2,
    )}\n`,
    "utf8",
  );
  writeFileSync(join(directory, "src", "main.ns"), source, "utf8");
}

function cleanBridgeEnv(extra = {}) {
  const env = { ...process.env };
  for (const key of [
    "ASTRA_NODE",
    "ASTRA_TSC_BIN",
    "ASTRA_COMPILER_BIN",
    "ASTRA_REPO_ROOT",
    "ASTRA_TARGET",
  ]) {
    delete env[key];
  }
  return { ...env, ...extra };
}

function withoutNodeEnv(extra = {}) {
  const env = cleanBridgeEnv({
    PATH: "",
    Path: "",
    ...extra,
  });
  delete env.ASTRA_CC;
  return env;
}

function findHostCompilerPath() {
  const names =
    platform === "win32"
      ? ["clang.exe", "gcc.exe", "cc.exe"]
      : ["clang", "gcc", "cc"];
  const pathValue = process.env.PATH ?? process.env.Path ?? "";
  const directories = [];
  for (const directory of pathValue.split(delimiter)) {
    if (!directory) continue;
    if (names.some((name) => existsSync(join(directory, name)))) {
      directories.push(directory);
    }
  }
  return directories.length > 0 ? directories.join(delimiter) : undefined;
}

function requireFile(path, label) {
  if (!existsSync(path)) {
    fail(`${label} produced no output at ${path}`);
  }
}

if (!skipBuild) {

  run(npm, ["run", "build"]);
  run("cargo", ["build", "--release", "-p", "astra-cli"]);
  run("cargo", [
    "build",
    "--release",
    "-p",
    "astra-compiler-api",
    "--bin",
    "astra-compiler",
  ]);
  run("cargo", ["build", "--release", "-p", "astra-nova", "--bin", "nova"]);
  run("cargo", ["build", "--release", "-p", "astra-nopm", "--bin", "nopm"]);
}

rmSync(outRoot, { recursive: true, force: true });
mkdirSync(binDir, { recursive: true });

copyFileSync(
  join(root, "target", "release", `astra-native${executable}`),
  join(binDir, `astra${executable}`),
);
if (platform !== "win32") {
  chmodSync(join(binDir, `astra${executable}`), 0o755);
}

const nopmLauncher = join(binDir, `nopm${executable}`);
copyFileSync(
  join(root, "target", "release", `nopm${executable}`),
  nopmLauncher,
);
if (platform !== "win32") {
  chmodSync(nopmLauncher, 0o755);
}

copyFileSync(
  join(root, "target", "release", `astra-compiler${executable}`),
  join(binDir, `astra-compiler${executable}`),
);
copyFileSync(
  join(root, "target", "release", `nova${executable}`),
  join(binDir, `nova${executable}`),
);

const bridgeRoot = join(outRoot, "lib", "cli");
mkdirSync(bridgeRoot, { recursive: true });
const { cpSync } = await import("node:fs");

copyFileSync(
  join(root, "orchestrator", "packages", "cli", "package.json"),
  join(bridgeRoot, "package.json"),
);
const bridgeDist = join(bridgeRoot, "dist");
mkdirSync(bridgeDist, { recursive: true });
cpSync(join(root, "orchestrator", "packages", "cli", "dist"), bridgeDist, {
  recursive: true,
});

const bridgeDeps = [
  "codegen-js",
  "codegen-ts",
  "compiler-client",
  "manifest",
  "toolchain",
];
for (const dep of bridgeDeps) {
  const depDir = join(bridgeRoot, "node_modules", "@neo-astra", dep);
  mkdirSync(depDir, { recursive: true });
  const depPkg = JSON.parse(
    readFileSync(join(root, "orchestrator", "packages", dep, "package.json"), "utf8"),
  );
  writeFileSync(
    join(depDir, "package.json"),
    `${JSON.stringify(depPkg, null, 2)}\n`,
    "utf8",
  );
  const depDist = join(depDir, "dist");
  mkdirSync(depDist, { recursive: true });
  cpSync(join(root, "orchestrator", "packages", dep, "dist"), depDist, {
    recursive: true,
  });
}

const externalDeps = ["@taplo/lib", "smol-toml", "typescript"];
for (const dep of externalDeps) {
  const src = join(root, "node_modules", ...dep.split("/"));
  const dst = join(bridgeRoot, "node_modules", ...dep.split("/"));
  if (existsSync(src)) {
    mkdirSync(dirname(dst), { recursive: true });
    cpSync(src, dst, { recursive: true });
  }
}

const typescriptPackage = `@typescript/typescript-${platform}-${arch}`;
const typescriptRoot = join(
  root,
  "node_modules",
  ...typescriptPackage.split("/"),
);
const typescriptOut = join(outRoot, "typescript");
const tscName = `tsc${executable}`;
if (!existsSync(join(typescriptRoot, "lib", tscName))) {
  fail(
    `missing ${typescriptPackage}; run npm install to fetch the pinned TypeScript native compiler`,
  );
}
mkdirSync(typescriptOut, { recursive: true });
copyFileSync(join(typescriptRoot, "lib", tscName), join(typescriptOut, tscName));

for (const entry of readdirSync(join(typescriptRoot, "lib"))) {
  if (entry === tscName) continue;
  const source = join(typescriptRoot, "lib", entry);
  const destination = join(typescriptOut, entry);

  if (statSync(source).isFile()) {
    copyFileSync(source, destination);
  }
}
const typescriptVersion = JSON.parse(
  readFileSync(join(root, "node_modules", "typescript", "package.json"), "utf8"),
).version;
writeFileSync(
  join(typescriptOut, "package.json"),
  `${JSON.stringify({ name: "typescript", version: typescriptVersion })}\n`,
  "utf8",
);
if (platform !== "win32") {
  chmodSync(join(typescriptOut, tscName), 0o755);
}

writeFileSync(
  join(outRoot, "README.txt"),
  [
    `Neo Astra ${version} (${platform} ${arch})`,
    "",
    `  bin/astra${executable}           the astra CLI and \`astra nopm\``,
    `  bin/nopm${executable}           the NoPM package manager launcher`,
    `  bin/astra-compiler${executable}  the native compiler`,
    `  bin/nova${executable}            the Nova development runtime`,
    "  typescript/              the pinned native TypeScript compiler",
    "",
    "Native/core commands need no Node.js, npm, or Cargo installation.",
    "JS/TS builds use the packaged Node bridge and need Node.js on PATH.",
    "Native builds still need a C compiler on PATH, or a pinned toolchain",
    "configured through build.json (astra downloads and verifies it).",
    "",
  ].join("\n"),
  "utf8",
);

for (const name of [`astra${executable}`, `nopm${executable}`, `nova${executable}`]) {
  if (platform !== "win32") {
    chmodSync(join(binDir, name), 0o755);
  }
}
if (platform !== "win32") {
  chmodSync(join(binDir, `astra-compiler${executable}`), 0o755);
}

const astraBinary = join(binDir, `astra${executable}`);
const smoke = spawnSync(astraBinary, ["--version"], { encoding: "utf8" });
if (smoke.status !== 0 || !smoke.stdout.includes(version)) {
  fail(`packaged astra did not answer --version: ${smoke.stderr || smoke.stdout}`);
}
const nopmSmoke = spawnSync(astraBinary, ["nopm", "--version"], {
  encoding: "utf8",
});
if (nopmSmoke.status !== 0 || !nopmSmoke.stdout.includes(version)) {
  fail(`packaged nopm did not answer --version: ${nopmSmoke.stderr || nopmSmoke.stdout}`);
}
const launcherSmoke = spawnSync(nopmLauncher, ["--version"], {
  encoding: "utf8",
});
if (launcherSmoke.status !== 0 || !launcherSmoke.stdout.includes(version)) {
  fail(
    `packaged nopm launcher did not answer --version: ${launcherSmoke.stderr || launcherSmoke.stdout}`,
  );
}

const smokeRoot = mkdtempSync(join(tmpdir(), "neo-astra-package-"));
try {
  const isolatedPackage = join(smokeRoot, "package");
  cpSync(outRoot, isolatedPackage, { recursive: true });
  const isolatedAstra = join(isolatedPackage, "bin", `astra${executable}`);
  const isolatedNopm = join(isolatedPackage, "bin", `nopm${executable}`);

  const nativeProject = join(smokeRoot, "native-project");
  const tsProject = join(smokeRoot, "ts-project");
  const jsProject = join(smokeRoot, "js-project");

  const nativeSource = [
    "pub fn main() -> i32 {",
    "    println(2 + 3)",
    "    return 0",
    "}",
    "",
  ].join("\n");
  const tsSource = [
    "{@script:ts as math;",
    "    @mode(core)",
    "    @exports { fn add(a: i32, b: i32) -> i32 }",
    "    @impl",
    "    export function add(a: number, b: number): number { return a + b; }",
    "@end",
    "}",
    "",
    "pub fn main() -> i32 {",
    "    println(math.add(2, 3))",
    "    return 0",
    "}",
    "",
  ].join("\n");
  const jsSource = [
    "{@script:js as math;",
    "    @mode(core)",
    "    @exports { fn add(a: i32, b: i32) -> i32 }",
    "    @impl",
    "    export function add(a, b) { return a + b; }",
    "@end",
    "}",
    "",
    "pub fn main() -> i32 {",
    "    println(math.add(2, 3))",
    "    return 0",
    "}",
    "",
  ].join("\n");

  writeSmokeProject(
    nativeProject,
    nativeSource,
    {
      c: { backend: "c", mode: "core", outDir: "dist/c" },
      native: { backend: "native", mode: "core", outDir: "dist/native" },
    },
    "native",
  );
  writeSmokeProject(
    tsProject,
    tsSource,
    { ts: { backend: "ts", mode: "core", outDir: "dist/ts" } },
    "ts",
  );
  writeSmokeProject(
    jsProject,
    jsSource,
    { js: { backend: "js", mode: "core", outDir: "dist/js" } },
    "js",
  );

  const cleanEnv = withoutNodeEnv();
  const versionResult = smokeRun(
    "packaged astra --version",
    isolatedAstra,
    ["--version"],
    { cwd: isolatedPackage, env: cleanEnv },
  );
  if (!versionResult.stdout.includes(version)) {
    fail(`packaged astra did not answer --version: ${versionResult.stdout}`);
  }
  const nopmVersion = smokeRun(
    "packaged nopm --version",
    isolatedNopm,
    ["--version"],
    { cwd: isolatedPackage, env: cleanEnv },
  );
  if (!nopmVersion.stdout.includes(version)) {
    fail(`packaged nopm did not answer --version: ${nopmVersion.stdout}`);
  }
  const dispatch = smokeRun(
    "packaged astra nopm version",
    isolatedAstra,
    ["nopm", "version"],
    { cwd: isolatedPackage, env: cleanEnv },
  );
  if (dispatch.stdout.trim() !== nopmVersion.stdout.trim()) {
    fail(
      `packaged astra nopm dispatch differs from nopm: ` +
        `${dispatch.stdout.trim()} !== ${nopmVersion.stdout.trim()}`,
    );
  }

  const check = smokeRun(
    "packaged astra check",
    isolatedAstra,
    ["check", nativeProject, "--json"],
    { cwd: isolatedPackage, env: cleanEnv },
  );
  if (!check.stdout.includes("\"manifest\"")) {
    fail(`packaged astra check produced an unexpected report: ${check.stdout}`);
  }
  smokeRun(
    "packaged astra fmt",
    isolatedAstra,
    ["fmt", nativeProject, "--check", "--json"],
    { cwd: isolatedPackage, env: cleanEnv },
  );
  smokeRun(
    "packaged astra ast",
    isolatedAstra,
    ["ast", nativeProject, "--json"],
    { cwd: isolatedPackage, env: cleanEnv },
  );
  smokeRun(
    "packaged astra explain",
    isolatedAstra,
    ["explain", nativeProject, "--json"],
    { cwd: isolatedPackage, env: cleanEnv },
  );
  smokeRun(
    "packaged C build",
    isolatedAstra,
    ["build", nativeProject, "--target", "c"],
    { cwd: isolatedPackage, env: cleanEnv },
  );
  requireFile(join(nativeProject, "dist", "c", "main.c"), "packaged C build");

  const compilerPath = findHostCompilerPath();
  if (compilerPath === undefined) {
    process.stdout.write(
      "[package] native executable smoke skipped: no clang/gcc/cc on PATH\n",
    );
  } else {
    const nativeEnv = withoutNodeEnv({
      PATH: compilerPath,
      Path: compilerPath,
    });
    smokeRun(
      "packaged native build",
      isolatedAstra,
      ["build", nativeProject, "--target", "native", "--no-download"],
      { cwd: isolatedPackage, env: nativeEnv },
    );
    const nativeExe = join(
      nativeProject,
      "dist",
      "native",
      platform === "win32" ? "main.exe" : "main",
    );
    requireFile(nativeExe, "packaged native build");
    const nativeRun = smokeRun(
      "packaged native executable",
      nativeExe,
      [],
      { cwd: dirname(nativeExe), env: nativeEnv },
    );
    if (!nativeRun.stdout.includes("5")) {
      fail(`packaged native executable produced unexpected output: ${nativeRun.stdout}`);
    }
  }

  const bridgeEnv = cleanBridgeEnv();
  smokeRun(
    "packaged TypeScript build",
    isolatedAstra,
    ["build", tsProject, "--target", "ts"],
    { cwd: isolatedPackage, env: bridgeEnv },
  );
  requireFile(
    join(tsProject, "dist", "ts", "src", "main.ts"),
    "packaged TypeScript build",
  );
  smokeRun(
    "packaged JavaScript build",
    isolatedAstra,
    ["build", jsProject, "--target", "js"],
    { cwd: isolatedPackage, env: bridgeEnv },
  );
  requireFile(
    join(jsProject, "dist", "js", "src", "main.js"),
    "packaged JavaScript build",
  );
  const jsRun = smokeRun(
    "packaged JavaScript output",
    process.execPath,
    [
      "-e",
      "import('./src/main.js').then((module) => { const status = module.main(); if (status !== 0) process.exit(status); })",
    ],
    { cwd: join(jsProject, "dist", "js"), env: bridgeEnv },
  );
  if (!jsRun.stdout.includes("5")) {
    fail(`packaged JavaScript output produced unexpected output: ${jsRun.stdout}`);
  }
} finally {
  rmSync(smokeRoot, { recursive: true, force: true });
}

const archive = join(
  root,
  "dist",
  `${releaseName}.${platform === "win32" ? "zip" : "tar.gz"}`,
);
rmSync(archive, { force: true });
const tar = platform === "win32"
  ? `${process.env["SystemRoot"] ?? "C:\\Windows"}\\System32\\tar.exe`
  : "tar";
run(
  tar,
  platform === "win32"
    ? ["-a", "-cf", archive, "-C", join(root, "dist"), releaseName]
    : ["-czf", archive, "-C", join(root, "dist"), releaseName],
);

const digest = createHash("sha256").update(readFileSync(archive)).digest("hex");
writeFileSync(
  `${archive}.sha256`,
  `${digest}  ${releaseName}.${platform === "win32" ? "zip" : "tar.gz"}\n`,
  "utf8",
);

process.stdout.write(
  `[package] ${releaseName}\n  ${archive}\n  sha256 ${digest}\n`,
);
