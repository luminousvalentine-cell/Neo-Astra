#!/usr/bin/env node

import { spawnSync } from "node:child_process";
import { createHash } from "node:crypto";
import {
  chmodSync,
  copyFileSync,
  existsSync,
  mkdirSync,
  mkdtempSync,
  readFileSync,
  rmSync,
  writeFileSync,
} from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { fileURLToPath } from "node:url";

const root = fileURLToPath(new URL("..", import.meta.url));
const skipBuild = process.argv.includes("--skip-build");
const version = JSON.parse(readFileSync(join(root, "package.json"), "utf8")).version;
const platform = process.platform;
const arch = process.arch;
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

if (!skipBuild) {
  run(platform === "win32" ? "npm.cmd" : "npm", ["run", "build"]);
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

const bun = join(root, "node_modules", "bun", "bin", "bun.exe");
if (!existsSync(bun)) {
  fail("Bun is not installed. Run npm install before packaging.");
}

run(bun, [
  "build",
  "scripts/bin/astra.ts",
  "--compile",
  "--minify",
  "--bytecode",
  "--sourcemap=none",
  "--outfile",
  join(binDir, `astra${executable}`),
]);

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
    "No Node.js, npm, or Cargo installation is required.",
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
  mkdirSync(join(smokeRoot, "src"), { recursive: true });
  writeFileSync(
    join(smokeRoot, "core.json"),
    `${JSON.stringify(
      {
        package: {
          name: "package-smoke",
          version: "0.0.0",
          edition: "2026",
          license: "MIT",
        },
      },
      null,
      2,
    )}\n`,
    "utf8",
  );
  writeFileSync(
    join(smokeRoot, "build.json"),
    `${JSON.stringify(
      {
        build: { defaultTarget: "ts", defaultMode: "core" },
        nova: { entry: "src/main.ns" },
        targets: {
          ts: { backend: "ts", mode: "core", outDir: "dist/ts" },
        },
      },
      null,
      2,
    )}\n`,
    "utf8",
  );
  writeFileSync(
    join(smokeRoot, "src", "main.ns"),
    [
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
    ].join("\n"),
    "utf8",
  );
  const smoke = spawnSync(
    astraBinary,
    ["build", smokeRoot, "--target", "ts"],
    { cwd: root, encoding: "utf8" },
  );
  if (smoke.status !== 0) {
    fail(
      `packaged TypeScript build failed: ${smoke.stderr || smoke.stdout}`,
    );
  }
  if (!existsSync(join(smokeRoot, "dist", "ts", "src", "main.ts"))) {
    fail("packaged TypeScript build produced no output");
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
