import { existsSync } from "node:fs";

import {
  NopmError,
  createDescriptor,
  formatPackageId,
  type DependencyRequest,
  type PackageDescriptor,
  type PackageId,
  parseDependencyRequest,
} from "./index.js";
import { packageDirectory, readProject, type ProjectManifest } from "./project.js";

export function resolvePathPackage(
  projectRoot: string,
  id: PackageId,
): PackageDescriptor {
  if (id.source !== "path" || id.path === null) {
    throw new NopmError("NP0011", "Expected a path package ID.", formatPackageId(id));
  }
  const directory = packageDirectory(projectRoot, id);
  if (!existsSync(directory)) {
    throw new NopmError(
      "NP0011",
      `Path package '${id.path}' does not exist.`,
      id.path ?? formatPackageId(id),
    );
  }

  let project: ProjectManifest;
  try {
    project = readProject(directory, { requireTargets: false });
  } catch (error) {
    throw new NopmError(
      "NP0011",
      `Unable to read path package '${formatPackageId(id)}': ${error instanceof Error ? error.message : String(error)}`,
      formatPackageId(id),
      { cause: error },
    );
  }

  const dependencies: Record<string, string> = {};
  for (const dependency of project.dependencies) {
    dependencies[dependency.alias] = dependency.request.spec;
  }

  const descriptorId = Object.freeze({
    ...id,
    name: project.packageName ?? id.name,
  });

  return createDescriptor({
    id: descriptorId,
    dependencies,
    targets: project.targets.map((target) => target.name),
    capabilities: project.capabilities,
    resolved: id.path,
    lifecycleScripts: false,
  });
}

export function pathDependencies(descriptor: PackageDescriptor): DependencyRequest[] {
  return Object.entries(descriptor.dependencies)
    .sort(([left], [right]) => left.localeCompare(right))
    .map(([alias, spec]) => parseDependencyRequest(alias, spec));
}
