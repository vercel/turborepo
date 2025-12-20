import type { Schema } from "@turbo/types";
import { findRootSync } from "@manypkg/find-root";
import json5 from "json5";
import { searchUp } from "./searchUp";

interface Options {
  cache?: boolean;
}

function contentCheck(content: string): boolean {
  const result: Schema | undefined = json5.parse(content);
  return !(result && "extends" in result);
}

const configCache: Record<string, string> = {};

export function clearTurboRootCache(): void {
  Object.keys(configCache).forEach((key) => {
    // eslint-disable-next-line @typescript-eslint/no-dynamic-delete -- This is safe.
    delete configCache[key];
  });
}

export function getTurboRoot(cwd?: string, opts?: Options): string | null {
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
