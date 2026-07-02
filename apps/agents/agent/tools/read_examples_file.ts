import { readFile } from "node:fs/promises";
import path from "node:path";

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
  async execute({ path: relativePath, maxLines }) {
    const repoRoot = await getRepoRoot();
    const filePath = await resolveExamplesFile(relativePath);
    const content = await readFile(filePath, "utf8");
    return {
      path: path.relative(repoRoot, filePath),
      content: content.split("\n").slice(0, maxLines).join("\n")
    };
  }
});
