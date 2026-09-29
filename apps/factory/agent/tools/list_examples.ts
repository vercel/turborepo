import { defineTool } from "eve/tools";
import { z } from "zod";

import {
  getRepoRoot,
  listExampleNames,
  resolveAutomatedExample
} from "../lib/repo.js";

export default defineTool({
  description:
    "List Turborepo examples available under the repository's examples/ directory.",
  inputSchema: z.object({}),
  async execute(_input, ctx) {
    const sandbox = await ctx.getSandbox();
    const examples = await listExampleNames(sandbox);
    const automatedExample = await resolveAutomatedExample(
      sandbox,
      ctx.session.auth.current,
      ctx.session.id
    );
    return {
      repoRoot: await getRepoRoot(sandbox),
      examples: automatedExample ? [automatedExample] : examples
    };
  }
});
