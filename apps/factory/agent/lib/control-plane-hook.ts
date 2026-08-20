import { defineHook } from "eve/hooks";

import {
  recordAgentRunModel,
  updateAgentRun,
  writeAgentRun
} from "./run-registry";

async function safely(task: Promise<void>): Promise<void> {
  try {
    await task;
  } catch (error) {
    console.error("Could not update the agent run registry.", error);
  }
}

export const controlPlaneHook = defineHook({
  events: {
    async "session.started"(event, ctx) {
      const now = new Date().toISOString();
      const agent =
        event.data.invocation?.name ??
        event.data.runtime?.agentName ??
        ctx.agent.nodeId ??
        ctx.agent.name;
      await safely(
        writeAgentRun({
          agent,
          id: ctx.session.id,
          sandbox: {
            id: ctx.session.id,
            provider: "eve",
            status: "running"
          },
          source: "eve",
          startedAt: now,
          status: "running",
          title: agent,
          trigger: ctx.channel.kind ?? "unknown",
          updatedAt: now
        })
      );
    },
    async "turn.started"(_event, ctx) {
      await safely(updateAgentRun(ctx.session.id, { status: "running" }));
    },
    async "step.started"(event, ctx) {
      // The first step of each turn is enough to catch the selected model and
      // any later change, without touching the ledger on every model call.
      if (event.data.stepIndex !== 0) return;
      await safely(recordAgentRunModel(ctx.session.id, event.data.modelId));
    },
    async "session.waiting"(_event, ctx) {
      await safely(updateAgentRun(ctx.session.id, { status: "waiting" }));
    },
    async "session.completed"(_event, ctx) {
      const now = new Date().toISOString();
      await safely(
        updateAgentRun(ctx.session.id, {
          finishedAt: now,
          sandbox: {
            id: ctx.session.id,
            provider: "eve",
            status: "stopped"
          },
          status: "completed"
        })
      );
    },
    async "session.failed"(_event, ctx) {
      const now = new Date().toISOString();
      await safely(
        updateAgentRun(ctx.session.id, {
          finishedAt: now,
          sandbox: {
            id: ctx.session.id,
            provider: "eve",
            status: "failed"
          },
          status: "failed"
        })
      );
    }
  }
});
