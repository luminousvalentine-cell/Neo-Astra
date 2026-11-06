export interface Plugin {
  readonly id: string;
  readonly name: string;
  readonly version: string;
}

export interface PackageRequest {
  readonly name: string;
  readonly source: string;
  readonly version?: string;
}

export interface ResolvedPackage {
  readonly name: string;
  readonly source: string;
  readonly version: string;
  readonly integrity?: string;
  readonly location: string;
}

export interface FetchedPackage {
  readonly resolved: ResolvedPackage;
  readonly root: string;
}

export interface PackageSourceAdapter extends Plugin {
  readonly kind: "package-source";
  resolve(request: PackageRequest): Promise<ResolvedPackage>;
  fetch(resolved: ResolvedPackage): Promise<FetchedPackage>;
}

export interface BackendRequest {
  readonly target: string;
  readonly mode: string;
  readonly ir: unknown;
}

export interface BackendResult {
  readonly output: string;
  readonly sourceMap?: string;
}

export interface BackendAdapter extends Plugin {
  readonly kind: "backend";
  readonly target: string;
  generate(request: BackendRequest): Promise<BackendResult>;
}

export class InMemoryPluginRegistry {
  readonly #packageSources = new Map<string, PackageSourceAdapter>();
  readonly #backends = new Map<string, BackendAdapter>();

  registerPackageSource(adapter: PackageSourceAdapter): void {
    if (this.#packageSources.has(adapter.id)) {
      throw new Error(`Duplicate package-source plugin: ${adapter.id}`);
    }
    this.#packageSources.set(adapter.id, adapter);
  }

  registerBackend(adapter: BackendAdapter): void {
    if (this.#backends.has(adapter.id)) {
      throw new Error(`Duplicate backend plugin: ${adapter.id}`);
    }
    this.#backends.set(adapter.id, adapter);
  }

  getPackageSource(id: string): PackageSourceAdapter | undefined {
    return this.#packageSources.get(id);
  }

  getBackend(id: string): BackendAdapter | undefined {
    return this.#backends.get(id);
  }

  listPackageSources(): readonly PackageSourceAdapter[] {
    return [...this.#packageSources.values()];
  }

  listBackends(): readonly BackendAdapter[] {
    return [...this.#backends.values()];
  }
}
