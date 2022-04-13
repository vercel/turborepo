import { execSync } from "child_process";
import { PackageManager } from "./types";

export const getPackageManagerVersion = (ws: PackageManager): string => {
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
