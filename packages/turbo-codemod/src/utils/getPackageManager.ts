import findUp from "find-up";
import path from "path";

export type PackageManager = "yarn" | "pnpm" | "npm";

const cache: { [cwd: string]: PackageManager } = {};

export default function getPackageManager({
  directory,
}: { directory?: string } = {}): PackageManager | undefined {
  const cwd = directory || process.cwd();
  if (cache[cwd]) {
    return cache[cwd];
  }

  const lockFile = findUp.sync(
    ["yarn.lock", "pnpm-lock.yaml", "package-lock.json"],
    {
      cwd,
    }
  );

  if (!lockFile) {
    return;
  }

  switch (path.basename(lockFile)) {
    case "yarn.lock":
      cache[cwd] = "yarn";
      break;

    case "pnpm-lock.yaml":
      cache[cwd] = "pnpm";
      break;

    case "package-lock.json":
      cache[cwd] = "npm";
      break;
  }

  return cache[cwd];
}
