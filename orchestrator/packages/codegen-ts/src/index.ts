import { spawnSync } from "node:child_process";
import { existsSync, mkdtempSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { createRequire } from "node:module";
import { tmpdir } from "node:os";
import { dirname, join } from "node:path";
import { generateModule } from "@neo-astra/codegen-js";
import type {
  CodegenDiagnostic,
  CodegenRequest,
  CodegenResult,
  TypeScriptTranspileOutcome,
} from "@neo-astra/codegen-js";

export const PINNED_TYPESCRIPT_VERSION = "7.0.2";

export interface TypeScriptCodegenRequest extends Omit<CodegenRequest, "backend"> {}

export type { CodegenDiagnostic, CodegenRequest, CodegenResult };
export { CodegenRequestError } from "@neo-astra/codegen-js";

const requireFromCodegenTs = createRequire(import.meta.url);

export function pinnedTypeScriptVersionError(
  installedVersion: string,
): string | undefined {
  if (installedVersion === PINNED_TYPESCRIPT_VERSION) {
    return undefined;
  }
  return `typescript@${PINNED_TYPESCRIPT_VERSION} is required, but typescript@${installedVersion} is installed`;
}

function resolveNativeTypeScript():
  | { readonly path: string; readonly error?: undefined }
  | { readonly path?: undefined; readonly error: string } {
  const executable = process.platform === "win32" ? "tsc.exe" : "tsc";
  const packagedDir = join(dirname(process.execPath), "typescript");
  const explicit = process.env["ASTRA_TSC_BIN"];
  let installedVersion: string | undefined;
  let tscPath: string | undefined;

  const readVersion = (packageJsonPath: string): string | undefined => {
    try {
      const value = JSON.parse(readFileSync(packageJsonPath, "utf8")) as {
        version?: unknown;
      };
      return typeof value.version === "string" ? value.version : undefined;
    } catch {
      return undefined;
    }
  };

  try {
    const packageJsonPath = requireFromCodegenTs.resolve("typescript/package.json");
    installedVersion = readVersion(packageJsonPath);
    const platformPackage = `@typescript/typescript-${process.platform}-${process.arch}`;
    const platformJson = requireFromCodegenTs.resolve(
      `${platformPackage}/package.json`,
    );
    tscPath = join(dirname(platformJson), "lib", executable);
  } catch {

  }

  const packagedJson = join(packagedDir, "package.json");
  const packagedVersion = existsSync(packagedJson)
    ? readVersion(packagedJson)
    : undefined;
  if (packagedVersion !== undefined) {
    installedVersion = packagedVersion;
  }
  const packagedTsc = join(packagedDir, executable);
  if (existsSync(packagedTsc)) {
    tscPath = packagedTsc;
  }
  if (explicit !== undefined && explicit.length > 0) {
    tscPath = explicit;
    if (installedVersion === undefined) {
      const version = spawnSync(explicit, ["--version"], {
        encoding: "utf8",
        windowsHide: true,
      });
      installedVersion =
        version.status === 0
          ? (version.stdout.trim().replace(/^Version\s+/i, "") || undefined)
          : undefined;
    }
  }

  if (installedVersion === undefined) {
    return {
      error: `the pinned TypeScript compiler (typescript@${PINNED_TYPESCRIPT_VERSION}) is not installed`,
    };
  }
  const versionError = pinnedTypeScriptVersionError(installedVersion);
  if (versionError !== undefined) {
    return { error: versionError };
  }
  if (tscPath === undefined || !existsSync(tscPath)) {
    return {
      error: `the pinned TypeScript native compiler (typescript@${PINNED_TYPESCRIPT_VERSION}) is not installed; install the platform package or set ASTRA_TSC_BIN`,
    };
  }
  return { path: tscPath };
}

export function transpileTypeScriptBody(
  paddedSource: string,
): TypeScriptTranspileOutcome {
  const tsc = resolveNativeTypeScript();
  if (tsc.error !== undefined) {
    return { ok: false, message: tsc.error };
  }

  const directory = mkdtempSync(join(tmpdir(), "astra-script-ts-"));
  try {
    const inputPath = join(directory, "script.ts");
    const outDir = join(directory, "out");
    writeFileSync(inputPath, paddedSource, "utf8");

    const result = spawnSync(
      tsc.path,
      [
        "--ignoreConfig",
        "--noCheck",
        "--target",
        "es2022",
        "--module",
        "esnext",
        "--moduleResolution",
        "bundler",
        "--sourceMap",
        "--outDir",
        outDir,
        "script.ts",
      ],
      { cwd: directory, encoding: "utf8" },
    );
    const output = `${result.stdout ?? ""}${result.stderr ?? ""}`;
    const diagnostics = output
      .split(/\r?\n/)
      .filter((line) => line.includes("error TS"));
    if (result.status !== 0 || diagnostics.length > 0) {
      return {
        ok: false,
        message:
          diagnostics.length > 0
            ? diagnostics.join("\n")
            : `tsc exited with status ${String(result.status)}`,
      };
    }
    const code = readFileSync(join(outDir, "script.js"), "utf8");
    let sourceMap: string;
    try {
      sourceMap = readFileSync(join(outDir, "script.js.map"), "utf8");
    } catch {
      return { ok: false, message: "the pinned TypeScript compiler emitted no source map" };
    }
    return { ok: true, code, sourceMap };
  } catch (error) {
    const message = error instanceof Error ? error.message : String(error);
    return { ok: false, message: `failed to run the pinned TypeScript compiler: ${message}` };
  } finally {
    rmSync(directory, { recursive: true, force: true });
  }
}

export function generateTypeScript(
  request: TypeScriptCodegenRequest,
): CodegenResult {
  return generateModule({
    transpileTypeScript: transpileTypeScriptBody,
    ...request,
    backend: "ts",
  });
}
