import type { WorkspaceInfo, FixtureInfo } from "./types";

interface NpmLockfile {
  name?: string;
  version?: string;
  lockfileVersion: number;
  packages: Record<string, NpmPackageEntry>;
}

interface NpmPackageEntry {
  name?: string;
  version?: string;
  resolved?: string;
  link?: boolean;
  dependencies?: Record<string, string>;
  devDependencies?: Record<string, string>;
  peerDependencies?: Record<string, string>;
  workspaces?: string[] | { packages: string[] };
}

/**
 * Determines the package name for a workspace entry in an npm lockfile.
 *
 * npm lockfile v2 stores workspace names in the symlink entries under
 * `node_modules/<name>` with `link: true` and `resolved: <path>`.
 * For v3, the workspace entry itself may have a `name` field.
 */
function resolveWorkspaceName(
  lockfile: NpmLockfile,
  workspacePath: string
): string {
  // Check if the workspace entry itself has a name (lockfileVersion 3)
  const entry = lockfile.packages[workspacePath];
  if (entry?.name) return entry.name;

  // Search for a node_modules symlink pointing to this workspace
  for (const [key, pkg] of Object.entries(lockfile.packages)) {
    if (pkg.link && pkg.resolved === workspacePath) {
      // key is like "node_modules/ui" or "node_modules/@scope/name"
      const name = key.replace(/^node_modules\//, "");
      if (name) return name;
    }
  }

  // Fall back to path basename
  return workspacePath.split("/").pop() || workspacePath;
}

function isWorkspacePath(key: string): boolean {
  return key !== "" && !key.includes("node_modules/");
}

export function parseNpmLockfile(
  content: string,
  filename: string,
  filepath: string
): FixtureInfo {
  const lockfile: NpmLockfile = JSON.parse(content);
  const workspaces: WorkspaceInfo[] = [];

  // Root workspace
  const root = lockfile.packages[""];
  if (root) {
    workspaces.push({
      path: ".",
      name: root.name || lockfile.name || "root",
      dependencies: root.dependencies || {},
      devDependencies: root.devDependencies || {},
      peerDependencies: root.peerDependencies || {}
    });
  }

  // Non-root workspace packages
  for (const [key, entry] of Object.entries(lockfile.packages)) {
    if (!isWorkspacePath(key)) continue;

    const name = resolveWorkspaceName(lockfile, key);
    workspaces.push({
      path: key,
      name,
      dependencies: entry.dependencies || {},
      devDependencies: entry.devDependencies || {},
      peerDependencies: entry.peerDependencies || {}
    });
  }

  // Determine workspace globs for root package.json
  let workspaceGlobs: string[] = [];
  if (root?.workspaces) {
    if (Array.isArray(root.workspaces)) {
      workspaceGlobs = root.workspaces;
    } else if (root.workspaces.packages) {
      workspaceGlobs = root.workspaces.packages;
    }
  }

  const lockfileVersion = String(lockfile.lockfileVersion);

  // npm v2 lockfiles work with npm 8+, v3 with npm 9+
  const npmVersion =
    lockfile.lockfileVersion >= 3 ? "npm@10.0.0" : "npm@8.19.0";

  return {
    filename,
    filepath,
    packageManager: "npm",
    lockfileName: "package-lock.json",
    frozenInstallCommand: ["npm", "ci"],
    workspaces,
    packageManagerVersion: npmVersion,
    hasPatches: false,
    patchFiles: [],
    lockfileVersion,
    rootExtras: {
      workspaces: workspaceGlobs
    }
  };
}
