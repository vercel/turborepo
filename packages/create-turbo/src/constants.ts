export type BaseVersions = "berry" | "yarn" | "pnpm" | "npm";
export type CommandName = "yarn" | "pnpm" | "npm";

export type PackageManager = {
  name: BaseVersions;
  command: CommandName;
  installArgs: string[];
  version: string;
};

export const PACKAGE_MANAGERS: Record<BaseVersions, PackageManager> = {
  npm: {
    name: "npm",
    command: "npm",
    installArgs: ["install"],
    version: "latest",
  },
  pnpm: {
    name: "pnpm",
    command: "pnpm",
    installArgs: ["install"],
    version: "latest",
  },
  yarn: {
    name: "yarn",
    command: "yarn",
    installArgs: ["install"],
    version: "1.x",
  },
  berry: {
    name: "berry",
    command: "yarn",
    installArgs: ["install", "--no-immutable"],
    version: "stable",
  },
};
