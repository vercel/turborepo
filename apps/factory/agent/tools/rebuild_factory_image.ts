import { defineTool } from "eve/tools";
import { z } from "zod";

import { triggerFactoryImageBuild } from "../lib/factory-image-trigger.js";
import { fetchMainCommit } from "../lib/github.js";
import { isOperatorChatPrincipal } from "../lib/operator-console.js";

export default defineTool({
  description:
    "Rebuild the shared factory sandbox image from the current main branch.",
  inputSchema: z.object({}),
  approval: ({ session }) =>
    isOperatorChatPrincipal(session.auth.current)
      ? "user-approval"
      : { type: "denied", reason: "Operator console access is required." },
  async execute(_input, ctx) {
    if (!isOperatorChatPrincipal(ctx.session.auth.current)) {
      throw new Error("Operator console access is required.");
    }
    return triggerFactoryImageBuild({
      commit: await fetchMainCommit(),
      ref: "refs/heads/main",
      trigger: "operator"
    });
  }
});
