import path from "node:path/posix";

import { defineTool } from "eve/tools";
import { z } from "zod";

import {
  fileExists,
  getExamplePath,
  getRepoRoot,
  listDirectory,
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
  async execute({ example }, ctx) {
    const sandbox = await ctx.getSandbox();
    const examplePath = await getExamplePath(sandbox, example);
    const repoRoot = await getRepoRoot(sandbox);
    const packageJson = await readJsonFile(
      sandbox,
      path.join(examplePath, "package.json")
    );
    const turboJsonPath = path.join(examplePath, "turbo.json");
    const entries = await listDirectory(sandbox, examplePath);

    return {
      example,
      path: path.relative(repoRoot, examplePath),
      packageManager: packageJson.packageManager,
      packageManagerName: packageManagerName(packageJson.packageManager),
      scripts: pickJsonObject(packageJson.scripts),
      dependencies: pickJsonObject(packageJson.dependencies),
      devDependencies: pickJsonObject(packageJson.devDependencies),
      workspaces: packageJson.workspaces ?? null,
      turboJson: (await fileExists(sandbox, turboJsonPath))
        ? await readJsonFile(sandbox, turboJsonPath)
        : null,
      readme: await readTextIfExists(
        sandbox,
        path.join(examplePath, "README.md"),
        80
      ),
      lockfiles: entries
        .filter(
          (entry) =>
            entry.type === "file" &&
            ["package-lock.json", "pnpm-lock.yaml", "yarn.lock"].includes(
              entry.name
            )
        )
        .map((entry) => entry.name)
        .sort(),
      topLevelDirectories: entries
        .filter((entry) => entry.type === "directory")
        .map((entry) => entry.name)
        .sort()
    };
  }
});
