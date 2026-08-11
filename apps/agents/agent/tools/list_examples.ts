import { defineTool } from "eve/tools";
import { z } from "zod";

import { getRepoRoot, listExampleNames } from "../lib/repo.js";

export default defineTool({
  description:
    "List Turborepo examples available under the repository's examples/ directory.",
  inputSchema: z.object({}),
  async execute(_input, ctx) {
    const sandbox = await ctx.getSandbox();
    return {
      repoRoot: await getRepoRoot(sandbox),
      examples: await listExampleNames(sandbox)
    };
  }
});
