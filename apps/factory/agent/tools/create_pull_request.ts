import { defineTool } from "eve/tools";
import { z } from "zod";

import { createPullRequest } from "../lib/create-pull-request.js";
import { isAppPrincipal } from "../lib/repo.js";
import { CONVENTIONAL_TITLE_PATTERN } from "../lib/pull-request.js";

const inputSchema = z.object({
  branchName: z
    .string()
    .regex(/^agents\/[A-Za-z0-9._/-]+$/, "Branch must start with agents/")
    .optional()
    .describe(
      "Branch for an interactive run. Automated runs derive an idempotent daily branch."
    ),
  body: z.string().default(""),
  title: z
    .string()
    .regex(
      CONVENTIONAL_TITLE_PATTERN,
      "Use 'type: Description' with an uppercase description and no scope."
    )
    .optional()
    .describe(
      "Required title for an interactive or automated performance pull request. Automated example maintenance titles itself."
    )
});

export default defineTool({
  description:
    "Create or update a draft vercel/turborepo pull request from validated sandbox changes. Automated example and performance runs enforce their own scope and evidence gates. An interactive run publishes every change in the checkout and needs an agents/* branch and a Conventional Commit title from the caller.",
  inputSchema,
  approval: ({ session }) =>
    isAppPrincipal(session.auth.current) ? "not-applicable" : "user-approval",
  async execute(input, ctx) {
    return createPullRequest(input, {
      auth: ctx.session.auth.current,
      sandbox: await ctx.getSandbox(),
      sessionId: ctx.session.id
    });
  }
});
