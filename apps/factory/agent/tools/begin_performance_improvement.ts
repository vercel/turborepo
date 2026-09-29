import { defineTool } from "eve/tools";
import { z } from "zod";

import { beginPerformanceRun } from "../lib/performance-validation.js";
import { isAppPrincipal } from "../lib/repo.js";

export default defineTool({
  description:
    "Initialize an automated daily performance-improvement run and return its required author and opposite reviewer models. Call this first.",
  inputSchema: z.object({}),
  async execute(_input, ctx) {
    if (!isAppPrincipal(ctx.session.auth.current)) {
      throw new Error("Daily performance improvements require app auth.");
    }
    return beginPerformanceRun(await ctx.getSandbox(), ctx.session.id);
  }
});
