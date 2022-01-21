import findUp from "find-up";
import path from "path";

export type WorkspaceImplementations = "yarn" | "pnpm" | "npm";

export interface ImplementationAndLockFile {
  implementation: WorkspaceImplementations | undefined;
  lockFile: string;
}
const cache: { [cwd: string]: ImplementationAndLockFile } = {};

export function getWorkspaceImplementationAndLockFile(
  cwd: string
):
  | { implementation: WorkspaceImplementations | undefined; lockFile: string }
  | undefined {
  if (cache[cwd]) {
    return cache[cwd];
  }

  const lockFile = findUp.sync(
    ["yarn.lock", "pnpm-workspace.yaml", "package-lock.json"],
    {
      cwd,
    }
  );

  if (!lockFile) {
    return;
  }

  switch (path.basename(lockFile)) {
    case "yarn.lock":
      cache[cwd] = {
        implementation: "yarn",
        lockFile,
      };
      break;

    case "pnpm-workspace.yaml":
      cache[cwd] = {
        implementation: "pnpm",
        lockFile,
      };
      break;

    case "package-lock.json":
      cache[cwd] = {
        implementation: "npm",
        lockFile,
      };
      break;
  }

  return cache[cwd];
}

export function getWorkspaceImplementation(
  cwd: string
): WorkspaceImplementations | undefined {
  return getWorkspaceImplementationAndLockFile(cwd)?.implementation;
}
