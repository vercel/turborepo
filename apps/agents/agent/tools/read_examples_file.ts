import path from "node:path/posix";

import { defineTool } from "eve/tools";
import { z } from "zod";

import { getRepoRoot, resolveExamplesFile } from "../lib/repo.js";

export default defineTool({
  description:
    "Read a repository file under examples/ by repository-relative path.",
  inputSchema: z.object({
    path: z
      .string()
      .min(1)
      .describe("Repository-relative file path under examples/."),
    maxLines: z.number().int().positive().max(1_000).default(200)
  }),
  async execute({ path: relativePath, maxLines }, ctx) {
    const sandbox = await ctx.getSandbox();
    const repoRoot = await getRepoRoot(sandbox);
    const filePath = await resolveExamplesFile(sandbox, relativePath);
    const content = await sandbox.readTextFile({
      path: filePath,
      startLine: 1,
      endLine: maxLines
    });
    if (content === null) {
      throw new Error(`${relativePath} does not exist.`);
    }
    return {
      path: path.relative(repoRoot, filePath),
      content
    };
  }
});
