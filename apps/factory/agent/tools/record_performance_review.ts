import { defineTool } from "eve/tools";
import { z } from "zod";

import { recordPerformanceReview } from "../lib/performance-validation.js";

export default defineTool({
  description:
    "Record the structured verdict returned by the required opposite-model performance reviewer for the exact current diff.",
  inputSchema: z.object({
    approved: z.boolean(),
    blockingFindings: z.array(z.string().min(1)),
    reviewer: z.enum([
      "fable_performance_reviewer",
      "gpt_performance_reviewer"
    ]),
    summary: z.string().min(1)
  }),
  async execute(review, ctx) {
    return recordPerformanceReview(
      await ctx.getSandbox(),
      ctx.session.id,
      review
    );
  }
});
