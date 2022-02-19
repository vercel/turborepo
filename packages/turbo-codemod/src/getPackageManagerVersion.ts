import { execSync } from "child_process";
import { WorkspaceImplementations } from "./getWorkspaceImplementation";

export const getPackageManagerVersion = (
  ws: WorkspaceImplementations
): string => {
  switch (ws) {
    case "yarn":
      return execSync("yarn --version").toString().trim();
    case "pnpm":
      return execSync("pnpm --version").toString().trim();
    case "npm":
      return execSync("npm --version").toString().trim();
    default:
      throw new Error(`${ws} is not supported`);
  }
};
