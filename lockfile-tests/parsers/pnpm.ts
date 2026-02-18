import yaml from "js-yaml";
import type { WorkspaceInfo, FixtureInfo } from "./types";

/**
 * pnpm lockfile structure varies across versions:
 *
 * v5.x (lockfileVersion 5.3/5.4): importers have `specifiers` as a separate
 * map, and `dependencies`/`devDependencies` map names to resolved versions.
 *
 * v6.x (lockfileVersion "6.0"/"6.1"): importers use inline objects with
 * `specifier` and `version` fields per dependency.
 *
 * v7+ (lockfileVersion "7.0"): same as v6 but packages are keyed without
 * leading `/`.
 *
 * v9 (lockfileVersion "9.0"): same as v7 structurally.
 */

interface PnpmLockfileV5 {
  lockfileVersion: number | string;
  importers: Record<string, PnpmImporterV5>;
  patchedDependencies?: Record<string, PnpmPatchInfo>;
  overrides?: Record<string, string>;
}

interface PnpmImporterV5 {
  specifiers?: Record<string, string>;
  dependencies?: Record<string, string>;
  devDependencies?: Record<string, string>;
}

interface PnpmCatalogEntry {
  specifier: string;
  version: string;
}

interface PnpmLockfileV6 {
  lockfileVersion: string;
  importers: Record<string, PnpmImporterV6>;
  patchedDependencies?: Record<string, PnpmPatchInfo>;
  overrides?: Record<string, string>;
  catalogs?: Record<string, Record<string, PnpmCatalogEntry>>;
  settings?: Record<string, unknown>;
}

interface PnpmImporterV6 {
  dependencies?: Record<string, PnpmDepEntry>;
  devDependencies?: Record<string, PnpmDepEntry>;
}

interface PnpmDepEntry {
  specifier: string;
  version: string;
}

interface PnpmPatchInfo {
  hash: string;
  path: string;
}

function isV5Format(lockfileVersion: number | string): boolean {
  if (typeof lockfileVersion === "number") return true;
  const num = parseFloat(lockfileVersion);
  return num < 6;
}

function extractV5Deps(
  importer: PnpmImporterV5,
  type: "dependencies" | "devDependencies"
): Record<string, string> {
  const result: Record<string, string> = {};
  const specifiers = importer.specifiers || {};
  const deps = importer[type] || {};

  for (const name of Object.keys(deps)) {
    result[name] = specifiers[name] || deps[name];
  }
  return result;
}

function extractV6Deps(
  deps: Record<string, PnpmDepEntry> | undefined
): Record<string, string> {
  if (!deps) return {};
  const result: Record<string, string> = {};
  for (const [name, entry] of Object.entries(deps)) {
    result[name] = entry.specifier;
  }
  return result;
}

function nameFromPath(workspacePath: string): string {
  if (workspacePath === ".") return "root";
  return workspacePath.split("/").pop() || workspacePath;
}

export function parsePnpmLockfile(
  content: string,
  filename: string,
  filepath: string
): FixtureInfo {
  const lockfile = yaml.load(content) as PnpmLockfileV5 | PnpmLockfileV6;
  const lockfileVersion = String(lockfile.lockfileVersion);
  const v5 = isV5Format(lockfile.lockfileVersion);

  const workspaces: WorkspaceInfo[] = [];

  if (v5) {
    const lf = lockfile as PnpmLockfileV5;
    for (const [path, importer] of Object.entries(lf.importers)) {
      workspaces.push({
        path: path === "." ? "." : path,
        name: nameFromPath(path),
        dependencies: extractV5Deps(importer, "dependencies"),
        devDependencies: extractV5Deps(importer, "devDependencies"),
        peerDependencies: {}
      });
    }
  } else {
    const lf = lockfile as PnpmLockfileV6;
    for (const [path, importer] of Object.entries(lf.importers)) {
      workspaces.push({
        path: path === "." ? "." : path,
        name: nameFromPath(path),
        dependencies: extractV6Deps(importer.dependencies),
        devDependencies: extractV6Deps(importer.devDependencies),
        peerDependencies: {}
      });
    }
  }

  // Ensure root workspace exists
  if (!workspaces.some((w) => w.path === ".")) {
    workspaces.unshift({
      path: ".",
      name: "root",
      dependencies: {},
      devDependencies: {},
      peerDependencies: {}
    });
  }

  // Extract patches
  const patches: string[] = [];
  const patchedDeps = (lockfile as PnpmLockfileV5).patchedDependencies || {};
  for (const info of Object.values(patchedDeps)) {
    if (info.path) patches.push(info.path);
  }

  // Determine appropriate pnpm version
  const numVersion =
    typeof lockfile.lockfileVersion === "number"
      ? lockfile.lockfileVersion
      : parseFloat(lockfileVersion);

  let pnpmVersion: string;
  if (numVersion >= 9) {
    pnpmVersion = "pnpm@9.15.0";
  } else if (numVersion >= 7) {
    pnpmVersion = "pnpm@8.15.0";
  } else if (numVersion >= 6) {
    pnpmVersion = "pnpm@8.15.0";
  } else {
    pnpmVersion = "pnpm@7.33.0";
  }

  // The pnpm-10-patch.lock fixture has lockfileVersion '9.0' but represents
  // pnpm 10 behavior. Detect this by filename convention.
  if (filename.startsWith("pnpm-10")) {
    pnpmVersion = "pnpm@10.0.0";
  }

  const rootExtras: Record<string, unknown> = {};
  if ((lockfile as PnpmLockfileV5).overrides) {
    rootExtras.pnpm = {
      overrides: (lockfile as PnpmLockfileV5).overrides
    };
  }

  // Extract catalogs for pnpm-workspace.yaml generation
  const lf6 = lockfile as PnpmLockfileV6;
  if (lf6.catalogs) {
    const catalogs: Record<string, Record<string, string>> = {};
    for (const [catalogName, entries] of Object.entries(lf6.catalogs)) {
      catalogs[catalogName] = {};
      for (const [pkg, entry] of Object.entries(entries)) {
        catalogs[catalogName][pkg] = entry.specifier;
      }
    }
    rootExtras.catalogs = catalogs;
  }

  // Extract settings for pnpm-workspace.yaml
  if (lf6.settings) {
    rootExtras.settings = lf6.settings;
  }

  return {
    filename,
    filepath,
    packageManager: "pnpm",
    lockfileName: "pnpm-lock.yaml",
    frozenInstallCommand: ["pnpm", "install", "--frozen-lockfile"],
    workspaces,
    packageManagerVersion: pnpmVersion,
    hasPatches: patches.length > 0,
    patchFiles: patches,
    lockfileVersion,
    rootExtras
  };
}
