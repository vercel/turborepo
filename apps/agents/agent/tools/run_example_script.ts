import path from "node:path";

import { defineTool } from "eve/tools";
import { z } from "zod";

import {
  getExamplePath,
  packageManagerName,
  pickJsonObject,
  readJsonFile,
  runCommand
} from "../lib/repo.js";

export default defineTool({
  description:
    "Run a package.json script in one example with its declared package manager for validation.",
  inputSchema: z.object({
    example: z
      .string()
      .min(1)
      .describe("Directory name under examples/, for example 'basic'."),
    script: z
      .string()
      .min(1)
      .describe(
        "package.json script name to run, for example 'build' or 'lint'."
      ),
    timeoutSeconds: z.number().int().positive().max(600).default(120)
  }),
  async execute({ example, script, timeoutSeconds }) {
    const examplePath = await getExamplePath(example);
    const packageJson = await readJsonFile(
      path.join(examplePath, "package.json")
    );
    const scripts = pickJsonObject(packageJson.scripts);
    if (!scripts || typeof scripts[script] !== "string") {
      throw new Error(
        `Example '${example}' does not define a '${script}' script.`
      );
    }

    const manager = packageManagerName(packageJson.packageManager) ?? "pnpm";
    return runCommand(
      manager,
      ["run", script],
      examplePath,
      timeoutSeconds * 1_000
    );
  }
});
