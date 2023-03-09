import { findRootSync } from "@manypkg/find-root";
import searchUp from "./searchUp";
import JSON5 from "json5";

interface Options {
  cache?: boolean;
}

function contentCheck(content: string): boolean {
  const result = JSON5.parse(content);
  return !result.extends;
}

const configCache: Record<string, string> = {};

function getTurboRoot(cwd?: string, opts?: Options): string | null {
  const cacheEnabled = opts?.cache ?? true;
  const currentDir = cwd || process.cwd();

  if (cacheEnabled && configCache[currentDir]) {
    return configCache[currentDir];
  }

  // Turborepo root can be determined by a turbo.json without an extends key
  let root = searchUp({
    target: "turbo.json",
    cwd: currentDir,
    contentCheck,
  });

  if (!root) {
    try {
      root = findRootSync(currentDir);
      if (!root) {
        return null;
      }
    } catch (err) {
      return null;
    }
  }

  if (cacheEnabled) {
    configCache[currentDir] = root;
  }

  return root;
}

export default getTurboRoot;
