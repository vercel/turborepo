import { defineHook } from "eve/hooks";

import { updateWorkspaceActivityForSession } from "../lib/workspace-store.js";
import { workspaceActionActivity } from "../lib/workspace.js";

async function updateActivity(sessionId: string, activity: string): Promise<void> {
  try {
    await updateWorkspaceActivityForSession(sessionId, activity);
  } catch (error) {
    // Dashboard updates must never fail the agent turn they are observing.
    console.error("Could not update Factory workspace activity.", error);
  }
}

export default defineHook({
  events: {
    "action.result"(_event, ctx) {
      return updateActivity(ctx.session.id, "Thinking");
    },
    "actions.requested"(event, ctx) {
      return updateActivity(
        ctx.session.id,
        workspaceActionActivity(event.data.actions)
      );
    },
    "approval.candidate"(event, ctx) {
      if (event.data.outcome !== "pending") return;
      return updateActivity(ctx.session.id, "Waiting for approval");
    },
    "input.requested"(_event, ctx) {
      return updateActivity(ctx.session.id, "Waiting for input");
    },
    "step.started"(_event, ctx) {
      return updateActivity(ctx.session.id, "Thinking");
    },
    "turn.completed"(_event, ctx) {
      return updateActivity(ctx.session.id, "Finishing");
    }
  }
});
