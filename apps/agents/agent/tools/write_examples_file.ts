import path from "node:path/posix";

import { defineTool } from "eve/tools";
import { z } from "zod";

import { resolveAutomatedExample, writeExamplesFile } from "../lib/repo.js";

export default defineTool({
  description:
    "Create or overwrite a repository file under examples/. Use this directly for example maintenance writes.",
  inputSchema: z.object({
    path: z
      .string()
      .min(1)
      .describe("Repository-relative file path under examples/."),
    content: z.string().describe("Complete file contents to write.")
  }),
  async execute({ path: relativePath, content }, ctx) {
    const sandbox = await ctx.getSandbox();
    const automatedExample = await resolveAutomatedExample(
      sandbox,
      ctx.session.auth.current,
      ctx.session.id
    );
    const normalizedPath = path.normalize(relativePath);
    if (
      automatedExample &&
      !normalizedPath.startsWith(`examples/${automatedExample}/`)
    ) {
      throw new Error(
        `Automated maintenance can only write examples/${automatedExample}/.`
      );
    }
    return writeExamplesFile(sandbox, normalizedPath, content);
  }
});
