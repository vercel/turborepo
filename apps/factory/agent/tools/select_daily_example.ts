import { defineTool } from "eve/tools";
import { z } from "zod";

import { resolveAutomatedSelection } from "../lib/repo.js";

export default defineTool({
  description:
    "Select today's single Turborepo example for automated maintenance. Always call this first during a scheduled or operator-triggered run, then inspect and update only the returned example.",
  inputSchema: z.object({}),
  async execute(_input, ctx) {
    const sandbox = await ctx.getSandbox();
    const auth = ctx.session.auth.current;
    if (!auth) throw new Error("Automated maintenance requires app auth.");
    return resolveAutomatedSelection(sandbox, auth, ctx.session.id);
  }
});
