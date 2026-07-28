import { defineTool } from "eve/tools";
import { z } from "zod";

import { getRepoRoot, listExampleNames } from "../lib/repo.js";

export default defineTool({
  description:
    "List Turborepo examples available under the repository's examples/ directory.",
  inputSchema: z.object({}),
  async execute() {
    return {
      repoRoot: await getRepoRoot(),
      examples: await listExampleNames()
    };
  }
});
