import path from "node:path";
import fs from "fs-extra";
import type { PackageManager } from "@turbo/utils";
import { TransformError } from "./errors";
import type { TransformInput, TransformResult } from "./types";

const meta = {
  name: "update-commands-in-readme"
};

const PACKAGE_MANAGERS: Array<PackageManager> = ["pnpm", "npm", "yarn", "bun"];

// Ordered longest-first so regex alternation matches "pnpm run" before bare "pnpm".
const PM_RUN_PATTERN = PACKAGE_MANAGERS.map((pm) => `${pm} run`).join("|");
const PM_BARE_PATTERN = PACKAGE_MANAGERS.join("|");

// Matches compound "<pm> run" commands inside word boundaries.
const PM_RUN_REGEX = new RegExp(`\\b(?:${PM_RUN_PATTERN})\\b`, "g");
// Matches bare "<pm>" not followed by " run" (negative lookahead prevents double-replacement).
const PM_BARE_REGEX = new RegExp(
  `\\b(?:${PM_BARE_PATTERN})\\b(?!\\s+run)`,
  "g"
);

// Matches fenced code blocks (``` ... ```) or inline code spans (` ... `).
// Fenced blocks are checked first to avoid partial backtick matches.
const CODE_REGION_REGEX = /```[\s\S]*?```|`[^`]+`/g;

/**
 * Replaces package manager command references inside markdown code spans and
 * fenced code blocks in README.md to match the user's selected package manager.
 */
export async function transform(args: TransformInput): TransformResult {
  const { prompts } = args;

  if (!prompts.packageManager) {
    return { result: "not-applicable", ...meta };
  }

  const readmeFilePath = path.join(prompts.root, "README.md");
  if (!fs.existsSync(readmeFilePath)) {
    return { result: "not-applicable", ...meta };
  }

  try {
    const data = await fs.readFile(readmeFilePath, "utf8");
    const updatedData = replacePackageManagerReferences(
      prompts.packageManager.name,
      data
    );
    await fs.writeFile(readmeFilePath, updatedData, "utf8");
  } catch (err) {
    throw new TransformError("Unable to update README.md", {
      transform: meta.name,
      fatal: false
    });
  }

  return { result: "success", ...meta };
}

/**
 * Within backtick-delimited code regions, replaces package manager references:
 * - "<pm> run" → "<selected> run"
 * - bare "<pm>" (not followed by "run") → "<selected>"
 */
export function replacePackageManagerReferences(
  targetPm: PackageManager,
  text: string
): string {
  return text.replace(CODE_REGION_REGEX, (codeBlock) => {
    // Pass 1: replace compound "<pm> run" → "<selected> run"
    let result = codeBlock.replace(PM_RUN_REGEX, `${targetPm} run`);
    // Pass 2: replace bare "<pm>" (not followed by "run") → "<selected>"
    result = result.replace(PM_BARE_REGEX, targetPm);
    return result;
  });
}
