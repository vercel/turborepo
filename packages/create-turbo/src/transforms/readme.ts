import path from "node:path";
import fs from "fs-extra";
import type { TransformInput, TransformResult } from "./types";

const meta = {
  name: "readme"
};

export async function transform(args: TransformInput): TransformResult {
  const { prompts } = args;
  const { root, packageManager } = prompts;

  if (!packageManager) {
    return { result: "not-applicable", ...meta };
  }

  const readmePath = path.join(root, "README.md");

  if (!fs.existsSync(readmePath)) {
    return { result: "not-applicable", ...meta };
  }

  let content = fs.readFileSync(readmePath, "utf-8");

  // Multiline regex to match the consecutive list of package manager execution commands
  // and replace the entire block with just the line matching the selected package manager.
  // Group 1: npx
  // Group 2: yarn
  // Group 3: pnpm
  content = content.replace(
    /(npx turbo.*)\r?\n(yarn (?:dlx|exec) turbo.*)\r?\n(pnpm exec turbo.*)/g,
    (match: string, npmCmd: string, yarnCmd: string, pnpmCmd: string) => {
      switch (packageManager.name) {
        case "npm":
          return npmCmd;
        case "yarn":
          return yarnCmd;
        case "pnpm":
          return pnpmCmd;
        case "bun":
          return npmCmd.replace("npx", "bunx");
        default:
          return match;
      }
    }
  );

  fs.writeFileSync(readmePath, content);

  return { result: "success", ...meta };
}
