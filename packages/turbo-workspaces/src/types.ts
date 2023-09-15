import type { PackageManager } from "@turbo/utils";
import type { Logger } from "./logger";

export interface Manager {
  name: PackageManager;
  lock: string;
}

export interface RequestedPackageManagerDetails {
  name: PackageManager;
  version?: string;
}

export interface AvailablePackageManagerDetails {
  name: PackageManager;
  version: string;
}

export interface Project {
  name: string;
  description?: string;
  packageManager: PackageManager;
  paths: {
    root: string;
    packageJson: string;
    lockfile: string;
    nodeModules: string;
    // pnpm workspace config file
    workspaceConfig?: string;
  };
  workspaceData: {
    globs: Array<string>;
    workspaces: Array<Workspace>;
  };
}

export interface Workspace {
  name: string;
  description?: string;
  paths: {
    root: string;
    packageJson: string;
    nodeModules: string;
  };
}

export type WorkspaceInfo = Pick<Workspace, "name" | "description">;

export interface DetectArgs {
  workspaceRoot: string;
}

export interface ReadArgs {
  workspaceRoot: string;
}

export interface CreateArgs {
  project: Project;
  to: AvailablePackageManagerDetails;
  logger: Logger;
  options?: Options;
}

export interface RemoveArgs {
  project: Project;
  to: AvailablePackageManagerDetails;
  logger: Logger;
  options?: Options;
}

export interface CleanArgs {
  project: Project;
  logger: Logger;
  options?: Options;
}

export interface ConvertArgs {
  project: Project;
  to: AvailablePackageManagerDetails;
  logger: Logger;
  options?: Options;
}

export interface InstallArgs {
  project: Project;
  to: RequestedPackageManagerDetails;
  logger?: Logger;
  options?: Options;
}

export interface Options {
  dry?: boolean;
  skipInstall?: boolean;
  interactive?: boolean;
}

export interface PackageManagerInstallDetails {
  name: string;
  template: string;
  command: PackageManager;
  installArgs: Array<string>;
  version: string;
  executable: string;
  semver: string;
  default?: boolean;
}

export type ManagerDetect = (args: DetectArgs) => Promise<boolean>;
export type ManagerRead = (args: ReadArgs) => Promise<Project>;
export type ManagerCreate = (args: CreateArgs) => Promise<void>;
export type ManagerRemove = (args: RemoveArgs) => Promise<void>;
export type ManagerClean = (args: CleanArgs) => Promise<void>;
export type ManagerConvert = (args: ConvertArgs) => Promise<void>;

export interface ManagerHandler {
  detect: ManagerDetect;
  read: ManagerRead;
  create: ManagerCreate;
  remove: ManagerRemove;
  clean: ManagerClean;
  convertLock: ManagerConvert;
}
