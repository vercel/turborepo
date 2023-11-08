import type { Schema } from "@turbo/types";
import { findRootSync } from "@manypkg/find-root";
import json5 from "json5";
import { searchUp } from "./searchUp";

interface Options {
  cache?: boolean;
}

function contentCheck(content: string): boolean {
  // eslint-disable-next-line import/no-named-as-default-member -- json5 exports different objects depending on if you're using esm or cjs (https://github.com/json5/json5/issues/240)
  const result: Schema | undefined = json5.parse(content);
  return !(result && "extends" in result);
}

const configCache: Record<string, string> = {};

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
