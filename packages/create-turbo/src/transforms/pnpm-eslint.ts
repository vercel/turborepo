import fs from "fs-extra";
import type { TransformInput, TransformResult } from "./types";

const meta = {
  name: "pnpm-eslint",
};

const VSCODE_ESLINT_CONFIG = {
  "eslint.workingDirectories": [{ mode: "auto" }],
};

export async function transform(args: TransformInput): TransformResult {
  const { project, prompts } = args;
  const { packageManager } = prompts;

  if (packageManager?.name === "pnpm") {
    // write the settings directory
    await fs.mkdir(`${project.paths.root}/.vscode`, { recursive: true });
    // write .vscode settings =- required for eslint plugin for work with pnpm workspaces
    await fs.writeJson(
      `${project.paths.root}/.vscode/settings.json`,
      VSCODE_ESLINT_CONFIG,
      {
        spaces: 2,
      }
    );
  } else {
    return { result: "not-applicable", ...meta };
  }

  return { result: "success", ...meta };
}
