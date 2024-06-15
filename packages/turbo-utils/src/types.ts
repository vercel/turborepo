import type { Schema } from "@turbo/types";

export type PackageManager = "npm" | "yarn" | "pnpm" | "bun";

export type DependencyList = Record<string, string>;

export interface DependencyGroups {
  dependencies?: DependencyList;
  devDependencies?: DependencyList;
  peerDependencies?: DependencyList;
  optionalDependencies?: DependencyList;
}

export interface PackageJson extends DependencyGroups {
  name: string;
  version: string;
  description?: string;
  private?: boolean;
  packageManager?: string;
  // there can be more in here, but we only care about packages
  workspaces?: Array<string> | { packages?: Array<string> };
  main?: string;
  module?: string;
  exports?: object;
  scripts?: Record<string, string>;
  turbo?: Schema;
}

export interface PNPMWorkspaceConfig {
  packages?: Array<string>;
}
