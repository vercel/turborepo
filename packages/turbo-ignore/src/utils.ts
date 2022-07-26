import fs from "fs";
import path from "path";
import { findRootSync } from "@manypkg/find-root";

type Scope = {
  scope: string | null;
  context: { path?: string };
};

export function searchUp({
  target,
  cwd,
}: {
  target: string;
  cwd: string;
}): string | null {
  const root = path.parse(cwd).root;

  let found = false;
  while (!found && cwd !== root) {
    if (fs.existsSync(path.join(cwd, target))) {
      found = true;
      break;
    }

    cwd = path.dirname(cwd);
  }

  if (found) {
    return cwd;
  }

  return null;
}

export function getScopeFromArgs({ args }: { args: Array<string> }): Scope {
  if (args.length && args[0] != null) {
    return { scope: args[0], context: {} };
  }
  return { scope: null, context: {} };
}

export function getScopeFromPath({ cwd }: { cwd: string }): Scope {
  const packageJsonPath = path.join(cwd, "package.json");
  try {
    const raw = fs.readFileSync(packageJsonPath, "utf8");
    const packageJsonContent: Record<string, string> & { name: string } =
      JSON.parse(raw);

    return {
      scope: packageJsonContent.name,
      context: { path: packageJsonPath },
    };
  } catch (e) {
    return { scope: null, context: {} };
  }
}

export function getTurboRoot(): string | null {
  // Turbo root can be determined by the presence of turbo.json
  let root = searchUp({ target: "turbo.json", cwd: process.cwd() });

  if (!root) {
    root = findRootSync(process.cwd());
    if (!root) {
      return null;
    }
  }
  return root;
}

export function getComparison(): null | {
  ref: string;
  type: "previousDeploy" | "headRelative";
} {
  if (process.env.VERCEL === "1") {
    if (process.env.VERCEL_GIT_PREVIOUS_SHA) {
      // use the commit SHA of the last successful deployment for this project / branch
      return {
        ref: process.env.VERCEL_GIT_PREVIOUS_SHA,
        type: "previousDeploy",
      };
    } else {
      return null;
    }
  }
  return { ref: "HEAD^", type: "headRelative" };
}
