export type CommandName = "init" | "check" | "write" | "candidates";

export interface ProjectReferencesOptions {
  cwd?: string;
  dryRun?: boolean;
  json?: boolean;
  verbose?: boolean;
  force?: boolean;
}

export interface ProjectReferencesResult {
  version: 1;
  command: CommandName;
  success: boolean;
  dryRun: boolean;
  changedFiles: Array<string>;
  diagnostics: Array<Diagnostic>;
  summary: {
    packageCount: number;
    validCount: number;
    excludedCount: number;
    ignoredCount: number;
    candidateCount: number;
  };
  candidates: Array<string>;
  newPackages: Array<string>;
}

export interface Diagnostic {
  level: "info" | "warning" | "error";
  code: string;
  message: string;
  path?: string;
  packagePath?: string;
  details?: Array<string>;
}

export interface WorkspacePackage {
  name: string;
  version: string;
  dir: string;
  relativePath: string;
  manifest: PackageManifest;
  hasTsconfig: boolean;
}

export interface PackageManifest {
  name?: string;
  version?: string;
  private?: boolean;
  dependencies?: Record<string, string>;
  devDependencies?: Record<string, string>;
  optionalDependencies?: Record<string, string>;
  peerDependencies?: Record<string, string>;
  workspaces?: Array<string> | { packages?: Array<string> };
  packageManager?: string;
}

export interface MigrationState {
  ignored: Array<string>;
  excluded: Array<string>;
}

export class ProjectReferencesError extends Error {
  readonly diagnostics: Array<Diagnostic>;

  constructor(message: string, diagnostics?: Array<Diagnostic>) {
    super(message);
    this.name = "ProjectReferencesError";
    this.diagnostics = diagnostics ?? [
      { level: "error", code: "error", message }
    ];
  }
}
