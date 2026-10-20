import fs from "node:fs";
import path from "node:path";
import type { Schema } from "@turbo/types";
import { findRootSync } from "@manypkg/find-root";
import json5 from "json5";

const TURBO_CONFIG_FILES = ["turbo.json", "turbo.jsonc"] as const;

interface Options {
  cache?: boolean;
}

function isRootTurboConfig(content: string): boolean {
  const result: Schema | undefined = json5.parse(content);
  return !(result && "extends" in result);
}

const configCache: Record<string, string> = {};

export function clearTurboRootCache(): void {
  for (const key of Object.keys(configCache)) {
    // eslint-disable-next-line @typescript-eslint/no-dynamic-delete -- This is safe.
    delete configCache[key];
  }
}

/**
 * Search upward from `cwd` for a directory containing turbo.json or turbo.jsonc
 * that is a root config (no "extends" key). Both filenames are checked at each
 * directory level so that a turbo.jsonc in a closer directory takes priority
 * over a turbo.json in a parent directory.
 */
function searchUpForTurboConfig(cwd: string): string | null {
  const fsRoot = path.parse(cwd).root;
  let dir = cwd;

  while (dir !== fsRoot) {
    for (const filename of TURBO_CONFIG_FILES) {
      try {
        const content = fs.readFileSync(path.join(dir, filename)).toString();
        if (isRootTurboConfig(content)) {
          return dir;
        }
      } catch {
        // file doesn't exist at this level, try next filename / parent
      }
    }
    dir = path.dirname(dir);
  }

  return null;
}

export function getTurboRoot(cwd?: string, opts?: Options): string | null {
  const cacheEnabled = opts?.cache ?? true;
  const currentDir = cwd || process.cwd();

  if (cacheEnabled && configCache[currentDir]) {
    return configCache[currentDir];
  }

  // Turborepo root can be determined by a turbo.json or turbo.jsonc without an extends key
  let root = searchUpForTurboConfig(currentDir);

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
