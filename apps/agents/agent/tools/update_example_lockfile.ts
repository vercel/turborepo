import path from "node:path";

import { defineTool } from "eve/tools";
import { z } from "zod";

import {
  getExamplePath,
  packageManagerName,
  readJsonFile,
  runCommand
} from "../lib/repo.js";

export default defineTool({
  description:
    "Update an example's lockfile programmatically by running its declared package manager install command.",
  inputSchema: z.object({
    example: z
      .string()
      .min(1)
      .describe("Directory name under examples/, for example 'basic'."),
    timeoutSeconds: z.number().int().positive().max(1200).default(300)
  }),
  async execute({ example, timeoutSeconds }) {
    const examplePath = await getExamplePath(example);
    const packageJson = await readJsonFile(
      path.join(examplePath, "package.json")
    );
    const manager = packageManagerName(packageJson.packageManager) ?? "pnpm";
    return runCommand(
      manager,
      ["install"],
      examplePath,
      timeoutSeconds * 1_000
    );
  }
});
