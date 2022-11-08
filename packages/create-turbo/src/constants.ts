export type CommandName = "yarn" | "pnpm" | "npm";

export type PackageManager = {
  name: string;
  template: string;
  command: CommandName;
  installArgs: string[];
  version: string;
  executable: string;
  semver: string;
};

export const PACKAGE_MANAGERS: Record<CommandName, PackageManager[]> = {
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
