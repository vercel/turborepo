import { defineTool } from "eve/tools";
import { z } from "zod";

import { alertUnsafeIssue } from "../lib/slack.js";
import {
  isAutomaticIssueSession,
  recordIssueAssessment
} from "../lib/issue-handling.js";

const inputSchema = z.discriminatedUnion("safe", [
  z.object({
    confidence: z.enum(["low", "medium", "high"]),
    confidenceReason: z.string().min(1),
    issueNumber: z.number().int().positive(),
    issueTitle: z.string().min(1),
    issueUrl: z.string().url(),
    safe: z.literal(true),
    securityReason: z.string().min(1)
  }),
  z.object({
    confidence: z.null(),
    confidenceReason: z.null(),
    issueNumber: z.number().int().positive(),
    issueTitle: z.string().min(1),
    issueUrl: z.string().url(),
    safe: z.literal(false),
    securityReason: z.string().min(1)
  })
]);

export default defineTool({
  description:
    "Record the mandatory security-triage and confidence result for an automatically opened GitHub issue. Unsafe results also alert Slack and thread the reason.",
  inputSchema,
  async execute(assessment, ctx) {
    if (!isAutomaticIssueSession(ctx.session.auth.current)) {
      throw new Error(
        "This tool is available only for automatic issue handling."
      );
    }
    const recorded = await recordIssueAssessment(
      await ctx.getSandbox(),
      ctx.session.id,
      assessment
    );
    if (recorded.safe) return { recorded, slack: null };

    const slack = await alertUnsafeIssue({
      issueNumber: recorded.issueNumber,
      issueTitle: recorded.issueTitle,
      issueUrl: recorded.issueUrl,
      reason: recorded.securityReason
    });
    if (!slack.ok) {
      throw new Error(
        `The issue was blocked, but Slack alerting failed: ${slack.error}`
      );
    }
    return { recorded, slack };
  }
});
