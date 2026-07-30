import path from "node:path";

import { defineTool } from "eve/tools";
import { z } from "zod";

import {
  findPackageJsonFiles,
  getRepoRoot,
  isJsonObject,
  readJsonFile
} from "../lib/repo.js";

const dependencyFields = [
  "dependencies",
  "devDependencies",
  "peerDependencies",
  "optionalDependencies"
] as const;

export default defineTool({
  description:
    "Find versions of one package across all package.json files under examples/.",
  inputSchema: z.object({
    packageName: z
      .string()
      .min(1)
      .describe("Package name to search for, for example 'turbo'.")
  }),
  async execute({ packageName }) {
    const repoRoot = await getRepoRoot();
    const packageJsonFiles = await findPackageJsonFiles(
      path.join(repoRoot, "examples")
    );
    const matches: Array<{ path: string; field: string; version: string }> = [];

    for (const packageJsonFile of packageJsonFiles) {
      const packageJson = await readJsonFile(packageJsonFile);
      for (const field of dependencyFields) {
        const dependencies = packageJson[field];
        if (!isJsonObject(dependencies)) {
          continue;
        }

        const version = dependencies[packageName];
        if (typeof version === "string") {
          matches.push({
            path: path.relative(repoRoot, packageJsonFile),
            field,
            version
          });
        }
      }
    }

    return { packageName, matches };
  }
});
