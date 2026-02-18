import type { WorkspaceInfo, FixtureInfo } from "./types";

interface BunLockfile {
  lockfileVersion: number;
  configVersion?: number;
  workspaces: Record<string, BunWorkspaceEntry>;
  packages?: Record<string, unknown>;
}

interface BunWorkspaceEntry {
  name?: string;
  version?: string;
  dependencies?: Record<string, string>;
  devDependencies?: Record<string, string>;
  peerDependencies?: Record<string, string>;
  catalog?: Record<string, string>;
}

/**
 * Bun lockfiles use a JSON format with trailing commas. Standard JSON.parse
 * will fail on these, so we strip trailing commas first.
 */
function parseJsonWithTrailingCommas(content: string): unknown {
  const cleaned = content.replace(/,(\s*[}\]])/g, "$1");
  return JSON.parse(cleaned);
}

function detectBunPatches(lockfile: BunLockfile): {
  hasPatches: boolean;
  patchFiles: string[];
} {
  // Bun patches appear as dependencies with "patch:" prefix or in patchedDependencies
  const patchFiles: string[] = [];
  const raw = lockfile as unknown as Record<string, unknown>;

  if (raw.patchedDependencies && typeof raw.patchedDependencies === "object") {
    for (const value of Object.values(
      raw.patchedDependencies as Record<string, string>
    )) {
      if (typeof value === "string") patchFiles.push(value);
    }
  }

  return {
    hasPatches: patchFiles.length > 0,
    patchFiles
  };
}

export function parseBunLockfile(
  content: string,
  filename: string,
  filepath: string
): FixtureInfo {
  const lockfile = parseJsonWithTrailingCommas(content) as BunLockfile;
  const workspaces: WorkspaceInfo[] = [];

  for (const [path, entry] of Object.entries(lockfile.workspaces)) {
    const wsPath = path === "" ? "." : path;
    const name =
      entry.name ||
      (wsPath === "." ? "root" : wsPath.split("/").pop() || wsPath);

    workspaces.push({
      path: wsPath,
      name,
      dependencies: entry.dependencies || {},
      devDependencies: entry.devDependencies || {},
      peerDependencies: entry.peerDependencies || {}
    });
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

  const patches = detectBunPatches(lockfile);

  // Derive workspace globs
  const workspacePaths = workspaces
    .filter((w) => w.path !== ".")
    .map((w) => w.path);
  const workspaceGlobs = deriveWorkspaceGlobs(workspacePaths);

  // configVersion 1 requires bun 1.3+, lockfileVersion 1 requires 1.2+, v0 needs 1.1.43+
  let bunVersion: string;
  if (lockfile.configVersion && lockfile.configVersion >= 1) {
    bunVersion = "bun@1.3.5";
  } else if (lockfile.lockfileVersion >= 1) {
    bunVersion = "bun@1.2.0";
  } else {
    bunVersion = "bun@1.1.43";
  }

  return {
    filename,
    filepath,
    packageManager: "bun",
    lockfileName: "bun.lock",
    frozenInstallCommand: ["bun", "install", "--frozen-lockfile"],
    workspaces,
    packageManagerVersion: bunVersion,
    hasPatches: patches.hasPatches,
    patchFiles: patches.patchFiles,
    lockfileVersion: String(lockfile.lockfileVersion),
    rootExtras: {
      workspaces: workspaceGlobs
    }
  };
}

function deriveWorkspaceGlobs(paths: string[]): string[] {
  const globs = new Set<string>();
  for (const p of paths) {
    const parts = p.split("/");
    if (parts.length === 2) {
      globs.add(`${parts[0]}/*`);
    } else if (parts.length === 1) {
      globs.add(p);
    } else {
      globs.add(`${parts[0]}/${parts[1]}/*`);
    }
  }
  return Array.from(globs);
}
