import path from "node:path/posix";

import { defineTool } from "eve/tools";
import { z } from "zod";

import { auditExampleTasks } from "../lib/example-tasks.js";
import {
  getExamplePath,
  readJsonFile,
  resolveAutomatedExample
} from "../lib/repo.js";

export default defineTool({
  description:
    "Inspect an example's turbo.json and root package scripts to identify every configured non-persistent Turbo task that should pass after updates.",
  inputSchema: z.object({
    example: z
      .string()
      .min(1)
      .describe("Directory name under examples/, for example 'basic'.")
  }),
  async execute({ example }, ctx) {
    const sandbox = await ctx.getSandbox();
    const effectiveExample =
      (await resolveAutomatedExample(
        sandbox,
        ctx.session.auth.current,
        ctx.session.id,
        example
      )) ?? example;
    const examplePath = await getExamplePath(sandbox, effectiveExample);
    const packageJson = await readJsonFile(
      sandbox,
      path.join(examplePath, "package.json")
    );
    const turboJson = await readJsonFile(
      sandbox,
      path.join(examplePath, "turbo.json")
    );

    return {
      example: effectiveExample,
      ...auditExampleTasks(packageJson, turboJson)
    };
  }
});
