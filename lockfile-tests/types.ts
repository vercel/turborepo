export type PackageManagerType = "npm" | "pnpm" | "yarn-berry" | "bun";

export interface TestCase {
  fixture: {
    filename: string;
    filepath: string;
    packageManager: PackageManagerType;
    lockfileName: string;
    frozenInstallCommand: string[];
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
  installSuccess: boolean;
  pruneOutput?: string;
  installOutput?: string;
  error?: string;
  durationMs: number;
}
