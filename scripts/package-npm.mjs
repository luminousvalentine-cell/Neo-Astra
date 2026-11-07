#!/usr/bin/env node

import {
  chmodSync,
  copyFileSync,
  existsSync,
  mkdirSync,
  readFileSync,
  rmSync,
  writeFileSync,
} from "node:fs";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const root = fileURLToPath(new URL("..", import.meta.url));
const version = JSON.parse(readFileSync(join(root, "package.json"), "utf8")).version;
const args = process.argv.slice(2);
const platform = process.platform;
const arch = process.arch;
const executable = platform === "win32" ? ".exe" : "";
const osName = { win32: "windows", linux: "linux", darwin: "darwin" }[platform];
const target = osName === undefined ? undefined : `neo-astra-${osName}-${arch}`;
const targets = [
  { name: "neo-astra-windows-x64", os: "win32", cpu: "x64" },
  { name: "neo-astra-linux-x64", os: "linux", cpu: "x64" },
  { name: "neo-astra-darwin-arm64", os: "darwin", cpu: "arm64" },
];
const supportedPlatform =
  target !== undefined &&
  targets.some(
    (entry) =>
      entry.name === target && entry.os === platform && entry.cpu === arch,
  );

function flagValue(flag) {
  const index = args.indexOf(flag);
  return index >= 0 ? args[index + 1] : undefined;
}

function fail(message) {
  process.stderr.write(`[package-npm] ${message}\n`);
  process.exit(1);
}

function copyFile(source, destination, mode) {
  if (!existsSync(source)) {
    fail(`missing ${source}; run npm run package first`);
  }
  mkdirSync(dirname(destination), { recursive: true });
  copyFileSync(source, destination);
  if (mode !== undefined && platform !== "win32") {
    chmodSync(destination, mode);
  }
}

function writeJson(path, value) {
  mkdirSync(dirname(path), { recursive: true });
  writeFileSync(path, `${JSON.stringify(value, null, 2)}\n`, "utf8");
}

const repository = {
  type: "git",
  url: "git+https://github.com/luminousvalentine-cell/Neo-Astra.git",
};
const common = {
  version,
  license: "Apache-2.0",
  repository,
  homepage: "https://github.com/luminousvalentine-cell/Neo-Astra#readme",
  bugs: {
    url: "https://github.com/luminousvalentine-cell/Neo-Astra/issues",
  },
  keywords: ["neo-astra", "language", "compiler", "nova", "nopm"],
  publishConfig: { access: "public" },
};

const outRoot = resolve(flagValue("--out") ?? join(root, "dist", "npm"));
const sourceDir = resolve(
  flagValue("--source") ??
    join(root, "dist", `neo-astra-${version}-${platform}-${arch}`),
);
const requestedPlatform = args.includes("--platform");
const requestedMeta = args.includes("--meta");
const buildPlatform = requestedPlatform || !requestedMeta;
const buildMeta = requestedMeta || !requestedPlatform;

if (buildPlatform) {
  if (!supportedPlatform || target === undefined) {
    fail(
      `no npm platform package is defined for ${platform}/${arch}; supported hosts are Windows x64, Linux x64, and macOS arm64`,
    );
  }
  if (!existsSync(sourceDir)) {
    fail(`missing ${sourceDir}; run npm run package first`);
  }

  const platformOut = join(outRoot, "platform");
  rmSync(platformOut, { recursive: true, force: true });
  mkdirSync(join(platformOut, "bin"), { recursive: true });
  mkdirSync(join(platformOut, "typescript"), { recursive: true });

  copyFile(
    join(sourceDir, "bin", `astra${executable}`),
    join(platformOut, "bin", `astra${executable}`),
    0o755,
  );
  copyFile(
    join(sourceDir, "bin", `astra-compiler${executable}`),
    join(platformOut, "bin", `astra-compiler${executable}`),
    0o755,
  );
  copyFile(
    join(sourceDir, "bin", `nova${executable}`),
    join(platformOut, "bin", `nova${executable}`),
    0o755,
  );
  copyFile(
    join(sourceDir, "bin", `nopm${executable}`),
    join(platformOut, "bin", `nopm${executable}`),
    0o755,
  );
  copyFile(
    join(sourceDir, "typescript", `tsc${executable}`),
    join(platformOut, "typescript", `tsc${executable}`),
    0o755,
  );
  copyFile(
    join(sourceDir, "typescript", "package.json"),
    join(platformOut, "typescript", "package.json"),
  );

  const host =
    platform === "win32"
      ? "Windows x64"
      : platform === "darwin"
        ? "macOS arm64"
        : "Linux x64";
  writeFileSync(
    join(platformOut, "README.md"),
    [
      `# ${target}`,
      "",
      `Native Neo Astra ${version} toolchain for ${host}. This package is`,
      "installed by the `neo-astra` meta package; it has no standalone entry",
      "points.",
      "",
      "The binaries are the same standalone tools as the GitHub release archive.",
      "No Node.js, npm, or Cargo is needed at runtime.",
      "",
    ].join("\n"),
    "utf8",
  );
  writeJson(join(platformOut, "package.json"), {
    ...common,
    name: target,
    description: `Neo Astra native toolchain for ${host}`,
    os: [platform],
    cpu: [arch],
    files: ["bin", "typescript", "README.md"],
  });
  process.stdout.write(
    `[package-npm] platform ${target} -> ${platformOut}\n`,
  );
}

if (buildMeta) {
  const metaOut = join(outRoot, "meta");
  rmSync(metaOut, { recursive: true, force: true });
  mkdirSync(join(metaOut, "bin"), { recursive: true });

  const placeholder = [
    "echo \"Error: neo-astra's postinstall script was not run.\" >&2",
    "echo \"\" >&2",
    "echo \"This happens with --ignore-scripts or a package manager that skips\" >&2",
    "echo \"install scripts. Reinstall without --ignore-scripts, or run:\" >&2",
    "echo \"  node node_modules/neo-astra/postinstall.mjs\" >&2",
    "exit 1",
    "",
  ].join("\n");
  for (const name of ["astra.exe", "nopm.exe", "nova.exe"]) {
    const path = join(metaOut, "bin", name);
    writeFileSync(path, placeholder, "utf8");
    if (platform !== "win32") {
      chmodSync(path, 0o755);
    }
  }

  copyFile(
    join(root, "scripts", "npm", "postinstall.mjs"),
    join(metaOut, "postinstall.mjs"),
  );
  const metaReadmePath = join(root, "scripts", "npm", "README.md");
  const metaReadme = existsSync(metaReadmePath)
    ? readFileSync(metaReadmePath, "utf8")
    : [
        "# Neo Astra",
        "",
        "Install the Neo Astra toolchain with npm:",
        "",
        "```sh",
        "npm install --global neo-astra",
        "astra --version",
        "nopm --version",
        "nova version",
        "```",
        "",
        "The npm package is an installer. The installed `astra`, `nopm`, and",
        "`nova` commands are the same native binaries as the GitHub archive;",
        "they do not need Node.js, npm, Cargo, or a source checkout at runtime.",
        "",
        "Do not install with `--ignore-scripts`.",
        "",
      ].join("\n");
  writeFileSync(join(metaOut, "README.md"), metaReadme, "utf8");

  const optionalDependencies = Object.fromEntries(
    targets.map((entry) => [entry.name, version]),
  );
  writeJson(join(metaOut, "package.json"), {
    ...common,
    name: "neo-astra",
    description: "Neo Astra language toolchain",
    bin: {
      astra: "./bin/astra.exe",
      nopm: "./bin/nopm.exe",
      nova: "./bin/nova.exe",
      "neo-astra": "./bin/astra.exe",
    },
    scripts: { postinstall: "node ./postinstall.mjs" },
    os: ["darwin", "linux", "win32"],
    cpu: ["x64", "arm64"],
    engines: { node: ">=18" },
    optionalDependencies,
    files: [
      "bin/astra.exe",
      "bin/nopm.exe",
      "bin/nova.exe",
      "postinstall.mjs",
      "README.md",
    ],
  });
  process.stdout.write(`[package-npm] meta neo-astra -> ${metaOut}\n`);
}
