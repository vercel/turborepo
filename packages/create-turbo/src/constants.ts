export type BaseVersions = "berry" | "yarn" | "pnpm" | "npm";
export type CommandName = "yarn" | "pnpm" | "npm";

export type PackageManager = {
  name: BaseVersions;
  command: CommandName;
};

export const PACKAGE_MANAGERS: Record<BaseVersions, PackageManager> = {
  npm: {
    name: "npm",
    command: "npm",
  },
  pnpm: {
    name: "pnpm",
    command: "pnpm",
  },
  yarn: {
    name: "yarn",
    command: "yarn",
  },
  berry: {
    name: "berry",
    command: "yarn",
  },
};
