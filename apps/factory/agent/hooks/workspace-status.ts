import { defineHook } from "eve/hooks";
import { toolResultFrom } from "eve/tools";

import {
  recordWorkspacePullRequestForSession,
  updateWorkspaceActivityForSession
} from "../lib/workspace-pull-request.js";
import createPullRequestTool from "../tools/create_pull_request.js";

async function update(
  sessionId: string,
  activity: string | undefined,
  startsTurn = false
): Promise<void> {
  try {
    await updateWorkspaceActivityForSession(sessionId, activity, startsTurn);
  } catch (error) {
    console.error("Could not update Factory workspace status.", error);
  }
}

export default defineHook({
  events: {
    "turn.started"(_event, ctx) {
      return update(ctx.session.id, "Thinking", true);
    },
    "actions.requested"(event, ctx) {
      const actions = event.data.actions;
      const activity =
        actions.length === 1 && actions[0]?.kind === "tool-call"
          ? `Running ${actions[0].toolName.replaceAll("_", " ")}`
          : `Running ${actions.length} actions`;
      return update(ctx.session.id, activity);
    },
    async "action.result"(event, ctx) {
      const result = toolResultFrom(event.data.result, createPullRequestTool);
      const output = result?.output;
      if (
        typeof output === "object" &&
        output !== null &&
        "number" in output &&
        typeof output.number === "number" &&
        "url" in output &&
        typeof output.url === "string"
      ) {
        await recordWorkspacePullRequestForSession(ctx.session.id, {
          number: output.number,
          url: output.url
        });
      }
      await update(ctx.session.id, "Thinking");
    },
    "approval.candidate"(event, ctx) {
      if (event.data.outcome === "pending")
        return update(ctx.session.id, "Waiting for approval");
    },
    "approval.settled"(_event, ctx) {
      return update(ctx.session.id, "Working");
    },
    "input.requested"(_event, ctx) {
      return update(ctx.session.id, "Waiting for input");
    },
    "turn.completed"(_event, ctx) {
      return update(ctx.session.id, undefined);
    },
    "turn.cancelled"(_event, ctx) {
      return update(ctx.session.id, undefined);
    },
    "turn.failed"(_event, ctx) {
      return update(ctx.session.id, undefined);
    }
  }
});
