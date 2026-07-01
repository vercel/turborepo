export type PackageManagerType = "npm" | "pnpm" | "yarn" | "yarn-berry" | "bun";

export interface TestCase {
  fixture: {
    filename: string;
    filepath: string;
    packageManager: PackageManagerType;
    lockfileName: string;
    packageManagerVersion: string;
    /**
     * Additionally assert that every package in the (pruned) lockfile can
     * resolve its declared dependencies to the correct version. Catches
     * "stranded" transitive deps that `npm ci --dry-run` silently accepts.
     */
    validateResolution?: boolean;
  };
  targetWorkspace: {
    name: string;
  };
  label: string;
  docker?: boolean;
  /** Run `turbo prune --production`. */
  production?: boolean;
  expectedFailure?: boolean;
}

export interface TestResult {
  label: string;
  success: boolean;
  pruneSuccess: boolean;
  validationSuccess: boolean;
  pruneOutput?: string;
  validationOutput?: string;
  error?: string;
  durationMs: number;
}
