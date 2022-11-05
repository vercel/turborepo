import { Logger } from "./logger";

export type PackageManagers = "npm" | "pnpm" | "yarn";
export type PackageManagerDetails = {
  name: PackageManagers;
  version: string;
};

export type Project = {
  name: string;
  packageManager: PackageManagers;
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
  paths: {
    root: string;
    packageJson: string;
  };
};

export type DependencyList = Record<string, string>;

export type PackageJson = {
  name?: string;
  workspaces?: Array<string>;
  packageManager?: string;
  devDependencies?: DependencyList;
  dependencies?: DependencyList;
};

export type VerifyArgs = {
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
  logger: Logger;
  options?: Options;
};

export type Options = {
  dry?: boolean;
  install?: boolean;
  interactive?: boolean;
};

export type PackageManager = {
  name: string;
  template: string;
  command: PackageManagers;
  installArgs: string[];
  version: string;
  executable: string;
  semver: string;
};

export const PACKAGE_MANAGERS: Record<PackageManagers, PackageManager[]> = {
  npm: [
    {
      name: "npm",
      template: "npm",
      command: "npm",
      installArgs: ["install"],
      version: "latest",
      executable: "npx",
      semver: "*",
    },
  ],
  pnpm: [
    {
      name: "pnpm6",
      template: "pnpm",
      command: "pnpm",
      installArgs: ["install"],
      version: "latest-6",
      executable: "pnpx",
      semver: "6.x",
    },
    {
      name: "pnpm",
      template: "pnpm",
      command: "pnpm",
      installArgs: ["install"],
      version: "latest",
      executable: "pnpm dlx",
      semver: ">=7",
    },
  ],
  yarn: [
    {
      name: "yarn",
      template: "yarn",
      command: "yarn",
      installArgs: ["install"],
      version: "1.x",
      executable: "npx",
      semver: "<2",
    },
    {
      name: "berry",
      template: "berry",
      command: "yarn",
      installArgs: ["install", "--no-immutable"],
      version: "stable",
      executable: "yarn dlx",
      semver: ">=2",
    },
  ],
};
