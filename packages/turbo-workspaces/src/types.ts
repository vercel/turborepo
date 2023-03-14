import { Logger } from "./logger";

export type PackageManager = "npm" | "pnpm" | "yarn";
export type PackageManagerDetails = {
  name: PackageManager;
  version?: string;
};

export type Project = {
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
};

export type Workspace = {
  name: string;
  description?: string;
  paths: {
    root: string;
    packageJson: string;
    nodeModules: string;
  };
};

export type WorkspaceInfo = Pick<Workspace, "name" | "description">;

export type DependencyList = Record<string, string>;

export type PackageJsonDependencies = {
  dependencies?: DependencyList;
  devDependencies?: DependencyList;
  peerDependencies?: DependencyList;
  optionalDependencies?: DependencyList;
};

export type PackageJson = PackageJsonDependencies & {
  name?: string;
  description?: string;
  workspaces?: Array<string>;
  packageManager?: string;
};

export type DetectArgs = {
  workspaceRoot: string;
};

export type ReadArgs = {
  workspaceRoot: string;
};

export type CreateArgs = {
  project: Project;
  to: PackageManagerDetails;
  logger: Logger;
  options?: Options;
};

export type RemoveArgs = {
  project: Project;
  to: PackageManagerDetails;
  logger: Logger;
  options?: Options;
};

export type CleanArgs = {
  project: Project;
  logger: Logger;
  options?: Options;
};

export type ConvertArgs = {
  project: Project;
  logger: Logger;
  options?: Options;
};

export type InstallArgs = {
  project: Project;
  to: PackageManagerDetails;
  logger?: Logger;
  options?: Options;
};

export type Options = {
  dry?: boolean;
  skipInstall?: boolean;
  interactive?: boolean;
};

export type PackageManagerInstallDetails = {
  name: string;
  template: string;
  command: PackageManager;
  installArgs: string[];
  version: string;
  executable: string;
  semver: string;
  default?: boolean;
};

export type ManagerDetect = (args: DetectArgs) => Promise<boolean>;
export type ManagerRead = (args: ReadArgs) => Promise<Project>;
export type ManagerCreate = (args: CreateArgs) => Promise<void>;
export type ManagerRemove = (args: RemoveArgs) => Promise<void>;
export type ManagerClean = (args: CleanArgs) => Promise<void>;
export type ManagerConvert = (args: ConvertArgs) => Promise<void>;

export type ManagerHandler = {
  detect: ManagerDetect;
  read: ManagerRead;
  create: ManagerCreate;
  remove: ManagerRemove;
  clean: ManagerClean;
  convertLock: ManagerConvert;
};
