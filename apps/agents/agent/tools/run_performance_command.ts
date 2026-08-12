import { defineTool } from "eve/tools";
import { z } from "zod";

import { recordCommandEvidence } from "../lib/performance-validation.js";
import { getRepoRoot, runCommand } from "../lib/repo.js";

export default defineTool({
  description:
    "Run one reproducible benchmark or correctness command in the Turborepo checkout and record its evidence for the current performance run.",
  inputSchema: z.object({
    command: z
      .string()
      .min(1)
      .regex(/^[A-Za-z0-9._+-]+$/),
    args: z.array(z.string()).max(30).default([]),
    phase: z.enum(["baseline", "after", "validation"]),
    timeoutSeconds: z.number().int().positive().max(1800).default(600)
  }),
  async execute({ command, args, phase, timeoutSeconds }, ctx) {
    const sandbox = await ctx.getSandbox();
    const evidence = await runCommand(
      sandbox,
      command,
      args,
      await getRepoRoot(sandbox),
      timeoutSeconds * 1_000
    );
    await recordCommandEvidence(sandbox, ctx.session.id, phase, evidence);
    return evidence;
  }
});
