import { execSync } from "child_process";
import { WorkspaceImplementations } from "./getWorkspaceImplementation";

export default function getPackageManagerVersion(
  ws: WorkspaceImplementations,
  root: string
): string {
  switch (ws) {
    case "yarn":
      return execSync("yarn --version", { cwd: root }).toString().trim();
    case "pnpm":
      return execSync("pnpm --version", { cwd: root }).toString().trim();
    case "npm":
      return execSync("npm --version", { cwd: root }).toString().trim();
    default:
      throw new Error(`${ws} is not supported`);
  }
}
