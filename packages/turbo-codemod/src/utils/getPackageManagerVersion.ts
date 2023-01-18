import { execSync } from "child_process";
import type { PackageManager } from "./getPackageManager";

export default function getPackageManagerVersion(
  packageManager: PackageManager,
  root: string
): string {
  switch (packageManager) {
    case "yarn":
      return execSync("yarn --version", { cwd: root }).toString().trim();
    case "pnpm":
      return execSync("pnpm --version", { cwd: root }).toString().trim();
    case "npm":
      return execSync("npm --version", { cwd: root }).toString().trim();
  }
}
