export type PackageManagerType = "npm" | "pnpm" | "yarn" | "yarn-berry" | "bun";

export interface TestCase {
  fixture: {
    filename: string;
    filepath: string;
    packageManager: PackageManagerType;
    lockfileName: string;
    packageManagerVersion: string;
  };
  targetWorkspace: {
    name: string;
  };
  label: string;
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
