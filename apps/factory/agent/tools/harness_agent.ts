import { defineTool } from "eve/tools";
import { z } from "zod";

import { runFactoryHarnessAgent } from "../lib/harness-agent.js";
import {
  isOperatorSessionPrincipal,
  selectedOperatorHarness,
  selectedOperatorModel
} from "../lib/operator-console.js";
import { DEFAULT_WORKSPACE_HARNESS } from "../lib/workspace.js";

export default defineTool({
  description:
    "Delegate repository inspection, code changes, and validation to the workspace's selected AI SDK HarnessAgent coding runtime. Use this for coding work in operator-console sessions, passing the maintainer's complete request and constraints.",
  inputSchema: z.object({
    prompt: z.string().min(1).max(20_000)
  }),
  approval: ({ session }) =>
    isOperatorSessionPrincipal(session.auth.current)
      ? "not-applicable"
      : {
          type: "denied",
          reason: "HarnessAgent is available only in operator workspaces."
        },
  async execute({ prompt }, ctx) {
    if (!isOperatorSessionPrincipal(ctx.session.auth.current)) {
      throw new Error("HarnessAgent is available only in operator workspaces.");
    }
    const harness =
      selectedOperatorHarness(ctx.session.auth.current) ??
      DEFAULT_WORKSPACE_HARNESS;
    return runFactoryHarnessAgent({
      abortSignal: ctx.abortSignal,
      harness,
      model: selectedOperatorModel(ctx.session.auth.current),
      prompt,
      sandbox: await ctx.getSandbox(),
      sessionId: `factory-${harness}-${ctx.session.id}`
    });
  }
});
