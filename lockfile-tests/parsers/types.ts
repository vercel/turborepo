export type PackageManagerType = "npm" | "pnpm" | "yarn-berry" | "bun";

export interface WorkspaceInfo {
  /** Relative path from repo root, e.g. "packages/a". Root workspace uses "." */
  path: string;
  /** Package name, e.g. "a" or "@repo/ui" */
  name: string;
  dependencies: Record<string, string>;
  devDependencies: Record<string, string>;
  peerDependencies: Record<string, string>;
}

export interface FixtureInfo {
  /** Original filename, e.g. "pnpm8.yaml" */
  filename: string;
  /** Absolute path to the fixture file */
  filepath: string;
  /** Which package manager this lockfile belongs to */
  packageManager: PackageManagerType;
  /** The filename the lockfile should have in a real repo */
  lockfileName: string;
  /** Frozen install command parts, e.g. ["pnpm", "install", "--frozen-lockfile"] */
  frozenInstallCommand: string[];
  /** All workspaces found in the lockfile (including root ".") */
  workspaces: WorkspaceInfo[];
  /** Package manager version string for corepack, e.g. "pnpm@9.0.0" */
  packageManagerVersion: string;
  /** Whether this fixture uses patches */
  hasPatches: boolean;
  /** Patch file paths referenced in the lockfile */
  patchFiles: string[];
  /** Specific lockfile version (from the lockfile itself) */
  lockfileVersion: string;
  /** Any extra root-level config needed (e.g. resolutions for yarn) */
  rootExtras: Record<string, unknown>;
}

export interface TestCase {
  fixture: FixtureInfo;
  /** The workspace to prune to */
  targetWorkspace: WorkspaceInfo;
  /** Human-readable label for logging */
  label: string;
  /** If true, failure is expected and verbose output is suppressed */
  expectedFailure?: boolean;
}

export interface TestResult {
  label: string;
  success: boolean;
  pruneSuccess: boolean;
  installSuccess: boolean;
  pruneOutput?: string;
  installOutput?: string;
  error?: string;
  durationMs: number;
}
