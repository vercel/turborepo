import * as fs from "node:fs";
import * as path from "node:path";
import type { Schema } from "@turbo/types";
import { findRootSync } from "@manypkg/find-root";
import json5 from "json5";
import { searchUp } from "./search-up";

interface Options {
  cache?: boolean;
}

function contentCheck(content: string): boolean {
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

export function getTurboRoot(cwd?: string, opts?: Options): string | null {
  const cacheEnabled = opts?.cache ?? true;
  const currentDir = cwd || process.cwd();

  if (cacheEnabled && configCache[currentDir]) {
    return configCache[currentDir];
  }

  const { root: filesystemRoot } = path.parse(currentDir);
  let root: string | null = null;
  let lastCwd = currentDir;

  while (lastCwd !== filesystemRoot) {
    const jsonPath = path.join(lastCwd, "turbo.json");
    const jsoncPath = path.join(lastCwd, "turbo.jsonc");

    if (fs.existsSync(jsonPath)) {
      try {
        const content = fs.readFileSync(jsonPath, "utf-8");
        if (contentCheck(content)) {
          root = lastCwd;
          break;
        }
      } catch {
        // ignore
      }
    }

    if (fs.existsSync(jsoncPath)) {
      try {
        const content = fs.readFileSync(jsoncPath, "utf-8");
        if (contentCheck(content)) {
          root = lastCwd;
          break;
        }
      } catch {
        // ignore
      }
    }

    lastCwd = path.dirname(lastCwd);
  }

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
