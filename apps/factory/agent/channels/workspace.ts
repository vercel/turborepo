import { randomUUID } from "node:crypto";

import { defineChannel, POST } from "eve/channels";

import { createWorkspace, mutateWorkspace } from "../lib/workspace-store.js";
import {
  isWorkspaceMutationRequest,
  parseCreateWorkspaceInput,
  toWorkspaceView,
  WORKSPACE_CREATE_ACTION,
  WORKSPACE_RUN_MODE,
  type WorkspaceRecord
} from "../lib/workspace.js";
import { operatorSessionPrincipal } from "../lib/operator-console.js";

type WorkspaceChannelState = {
  workspaceId: string;
};

type WorkspaceReceiveTarget = {
  mode: "conversation" | "task";
  title: string;
  workspaceId: string;
};

export default defineChannel<
  WorkspaceChannelState,
  { state: WorkspaceChannelState },
  WorkspaceReceiveTarget
>({
  state: { workspaceId: "" },
  context: (state) => ({ state }),
  metadata: (state) => ({ workspaceId: state.workspaceId }),
  async receive({ auth, message, target }, { from }) {
    return from(target.workspaceId).send(message, {
      auth,
      mode: target.mode,
      state: { workspaceId: target.workspaceId },
      title: target.title
    });
  },
  routes: [
    POST("/eve/v1/workspaces", async (request, { from }) => {
      if (!isWorkspaceMutationRequest(request, WORKSPACE_CREATE_ACTION)) {
        return Response.json(
          { error: "Invalid operator request." },
          { status: 403 }
        );
      }

      const input = parseCreateWorkspaceInput(
        await request.json().catch(() => null)
      );
      if (!input?.prompt) {
        return Response.json(
          { error: "A prompt is required." },
          { status: 400 }
        );
      }

      const id = `ws_${randomUUID().replaceAll("-", "")}`;
      const turnId = `turn_${randomUUID().replaceAll("-", "")}`;
      const now = new Date().toISOString();
      const workspace: WorkspaceRecord = {
        agent: "eve",
        activeTurnId: turnId,
        createdAt: now,
        harness: input.harness,
        id,
        messages: [
          { createdAt: now, id: turnId, role: "user", text: input.prompt }
        ],
        model: input.model,
        sandbox: { provider: "vercel", status: "running" },
        status: "running",
        title: input.title,
        updatedAt: now,
        version: 2
      };
      await createWorkspace(workspace);

      try {
        const session = await from(id).send(input.prompt, {
          auth: operatorSessionPrincipal(input.model, input.harness),
          mode: WORKSPACE_RUN_MODE,
          state: { workspaceId: id },
          title: input.title
        });
        const saved = await mutateWorkspace(id, (current) => ({
          ...current,
          sessionId: session.id
        }));
        return Response.json(toWorkspaceView(saved), {
          status: 202,
          headers: { "cache-control": "no-store" }
        });
      } catch (error) {
        const reason =
          error instanceof Error && error.message.trim()
            ? `Could not start the workspace session: ${error.message.trim()}`
            : "Could not start the workspace session.";
        await failWorkspace(id, reason);
        throw error;
      }
    })
  ],
  events: {
    async "turn.started"(event, channel, ctx) {
      const workspaceId = channel.state.workspaceId;
      if (!workspaceId) return;
      const sandbox = await ctx.getSandbox();
      await mutateWorkspace(workspaceId, (workspace) => ({
        ...workspace,
        activeTurnId: event.turnId,
        error: undefined,
        sandbox: {
          id: sandbox.id,
          provider: "vercel",
          status: "running"
        },
        sessionId: ctx.session.id,
        status: "running",
        updatedAt: new Date().toISOString()
      })).catch(() => undefined);
    },
    async "message.completed"(event, channel) {
      const workspaceId = channel.state.workspaceId;
      if (!workspaceId || !event.message) return;
      const message = event.message;
      const now = new Date().toISOString();
      await mutateWorkspace(workspaceId, (workspace) => ({
        ...workspace,
        messages: [
          ...workspace.messages,
          {
            createdAt: now,
            id: `msg_${event.turnId}_${event.sequence}`,
            role: "assistant" as const,
            text: message.slice(0, 100_000)
          }
        ].slice(-1000),
        updatedAt: now
      })).catch(() => undefined);
    },
    async "turn.completed"(_event, channel) {
      await settleWorkspace(channel.state.workspaceId);
    },
    async "turn.cancelled"(_event, channel) {
      await settleWorkspace(channel.state.workspaceId);
    },
    async "turn.failed"(event, channel) {
      await failWorkspace(channel.state.workspaceId, event.message);
    },
    async "session.failed"(event, channel) {
      await failWorkspace(channel.state.workspaceId, event.message);
    }
  }
});

async function settleWorkspace(workspaceId: string): Promise<void> {
  if (!workspaceId) return;
  const now = new Date().toISOString();
  await mutateWorkspace(workspaceId, (workspace) => ({
    ...workspace,
    activeTurnId: undefined,
    error: undefined,
    status: "idle",
    updatedAt: now
  })).catch(() => undefined);
}

async function failWorkspace(
  workspaceId: string,
  message: string
): Promise<void> {
  if (!workspaceId) return;
  const now = new Date().toISOString();
  await mutateWorkspace(workspaceId, (workspace) => ({
    ...workspace,
    activeTurnId: undefined,
    error: message.slice(0, 2000),
    sandbox: { ...workspace.sandbox, status: "error" },
    status: "error",
    updatedAt: now
  })).catch(() => undefined);
}
