import { existsSync, mkdirSync, rmSync } from "node:fs";
import { dirname, join } from "node:path";

import {
  NopmError,
  createDescriptor,
  formatPackageId,
  parseDependencyRequest,
  type DependencyRequest,
  type PackageDescriptor,
  type PackageId,
} from "./index.js";
import { defaultCommandRunner, type CommandRunner } from "./command.js";
import { readProject, type ProjectManifest } from "./project.js";
import type { AdapterResolution } from "./npm-adapter.js";

export function resolveGitPackage(
  id: PackageId,
  alias: string,
  cacheRoot: string,
  runner: CommandRunner = defaultCommandRunner,
): AdapterResolution {
  if (id.source !== "git" || id.reference === null) {
    throw new NopmError("NP0011", "Expected a git package ID with a reference.", formatPackageId(id));
  }

  const commit = resolveCommit(id.name, id.reference, runner);
  const packageDirectory = join(cacheRoot, "git", commit);

  if (!existsSync(packageDirectory)) {
    clone(urlOf(id), commit, packageDirectory, runner);
  }

  let project: ProjectManifest;
  try {
    project = readProject(packageDirectory, { requireTargets: false });
  } catch (error) {
    throw new NopmError(
      "NP0011",
      `Unable to read git package '${formatPackageId(id)}': ${error instanceof Error ? error.message : String(error)}`,
      formatPackageId(id),
      { cause: error },
    );
  }

  const dependencies: Record<string, string> = {};
  for (const dependency of project.dependencies) {
    dependencies[dependency.alias] = dependency.request.spec;
  }

  const descriptor = createDescriptor({
    id,
    dependencies,
    targets: project.targets.map((target) => target.name),
    capabilities: project.capabilities,
    integrity: `git:${commit}`,
    resolved: urlOf(id),
    commit,
    lifecycleScripts: false,
  });

  return Object.freeze({
    descriptor,
    request: parseDependencyRequest(alias, formatPackageId(id)),
  });
}

function resolveCommit(url: string, reference: string, runner: CommandRunner): string {
  if (/^[0-9a-f]{40}$/i.test(reference)) {
    return reference.toLowerCase();
  }

  const result = runner.run("git", ["ls-remote", url, reference]);
  if (result.status !== 0) {
    throw new NopmError(
      "NP0011",
      `Unable to resolve git reference '${reference}': ${result.stderr.trim()}`,
    );
  }

  const lines = result.stdout
    .split(/\r?\n/)
    .map((line) => line.trim())
    .filter((line) => line.length > 0);
  if (lines.length === 0) {
    throw new NopmError("NP0011", `Git reference '${reference}' was not found in '${url}'.`);
  }

  const peeled = lines.find((line) => line.endsWith("^{}"));
  const selected = peeled ?? lines[0];
  const commit = selected?.split(/\s+/)[0];
  if (commit === undefined || !/^[0-9a-f]{40}$/i.test(commit)) {
    throw new NopmError("NP0011", `Git returned an invalid commit for '${reference}'.`);
  }
  return commit.toLowerCase();
}

function clone(
  url: string,
  commit: string,
  packageDirectory: string,
  runner: CommandRunner,
): void {
  mkdirSync(dirname(packageDirectory), { recursive: true });
  const cloneResult = runner.run("git", ["clone", "--quiet", url, packageDirectory]);
  if (cloneResult.status !== 0) {
    rmSync(packageDirectory, { recursive: true, force: true });
    throw new NopmError(
      "NP0011",
      `Unable to clone '${url}': ${cloneResult.stderr.trim()}`,
    );
  }
  const checkout = runner.run("git", ["-C", packageDirectory, "checkout", "--quiet", "--detach", commit]);
  if (checkout.status !== 0) {
    rmSync(packageDirectory, { recursive: true, force: true });
    throw new NopmError(
      "NP0011",
      `Unable to check out git commit '${commit}': ${checkout.stderr.trim()}`,
    );
  }
}

function urlOf(id: PackageId): string {
  return id.name;
}
