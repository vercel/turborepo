import { existsSync } from "node:fs";
import { readdir } from "node:fs/promises";
import path from "node:path";

import { defineTool } from "eve/tools";
import { z } from "zod";

import {
  getExamplePath,
  packageManagerName,
  pickJsonObject,
  readJsonFile,
  readTextIfExists
} from "../lib/repo.js";

export default defineTool({
  description:
    "Inspect one example's package metadata, Turbo config, README excerpt, lockfiles, and workspace shape.",
  inputSchema: z.object({
    example: z
      .string()
      .min(1)
      .describe("Directory name under examples/, for example 'basic'.")
  }),
  async execute({ example }) {
    const examplePath = await getExamplePath(example);
    const packageJson = await readJsonFile(
      path.join(examplePath, "package.json")
    );
    const turboJsonPath = path.join(examplePath, "turbo.json");
    const entries = await readdir(examplePath, { withFileTypes: true });

    return {
      example,
      path: path.relative(process.cwd(), examplePath),
      packageManager: packageJson.packageManager,
      packageManagerName: packageManagerName(packageJson.packageManager),
      scripts: pickJsonObject(packageJson.scripts),
      dependencies: pickJsonObject(packageJson.dependencies),
      devDependencies: pickJsonObject(packageJson.devDependencies),
      workspaces: packageJson.workspaces ?? null,
      turboJson: existsSync(turboJsonPath)
        ? await readJsonFile(turboJsonPath)
        : null,
      readme: await readTextIfExists(path.join(examplePath, "README.md"), 80),
      lockfiles: entries
        .filter(
          (entry) =>
            entry.isFile() &&
            ["package-lock.json", "pnpm-lock.yaml", "yarn.lock"].includes(
              entry.name
            )
        )
        .map((entry) => entry.name)
        .sort(),
      topLevelDirectories: entries
        .filter((entry) => entry.isDirectory())
        .map((entry) => entry.name)
        .sort()
    };
  }
});
