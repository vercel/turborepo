import { randomUUID } from "node:crypto";

import { getRun, start } from "workflow/api";

import { workspaceTurnWorkflow } from "../../workflows/workspace-turn";
import {
  beginWorkspaceTurn,
  DEFAULT_WORKSPACE_MODEL,
  recoverTerminalWorkspaceTurn,
  recordWorkspaceWorkflowRun,
  toWorkspaceView,
  workspaceSandboxName,
  type WorkspaceModel,
  type WorkspaceRecord,
  type PublicWorkspaceView
} from "./workspace";
import {
  createWorkspace,
  getWorkspace,
  mutateWorkspace
} from "./workspace-store";

export async function createFxWorkspace(input: {
  readonly model?: WorkspaceModel;
  readonly prompt?: string;
  readonly title: string;
}): Promise<PublicWorkspaceView> {
  const id = `ws_${randomUUID().replaceAll("-", "")}`;
  const now = new Date().toISOString();
  const workspace: WorkspaceRecord = {
    agent: "fx",
    createdAt: now,
    id,
    messages: [],
    model: input.model ?? DEFAULT_WORKSPACE_MODEL,
    publishToken: randomUUID(),
    sandbox: {
      name: workspaceSandboxName(id),
      provider: "vercel",
      status: "pending"
    },
    status: "idle",
    title: input.title,
    updatedAt: now,
    version: 1
  };
  await createWorkspace(workspace);
  return input.prompt
    ? queueWorkspaceTurn(workspace.id, input.prompt)
    : toWorkspaceView(workspace);
}

export async function queueWorkspaceTurn(
  workspaceId: string,
  message: string
): Promise<PublicWorkspaceView> {
  const current = await getWorkspace(workspaceId);
  if (current?.status === "running" && current.workflowRunId) {
    const expectedWorkflowRunId = current.workflowRunId;
    const workflowStatus = await getRun(expectedWorkflowRunId).status;
    if (["cancelled", "completed", "failed"].includes(workflowStatus)) {
      const recoveredAt = new Date().toISOString();
      await mutateWorkspace(workspaceId, (workspace) =>
        recoverTerminalWorkspaceTurn(
          workspace,
          expectedWorkflowRunId,
          workflowStatus,
          recoveredAt
        )
      );
    }
  }
  const turnId = `turn_${randomUUID().replaceAll("-", "")}`;
  const createdAt = new Date().toISOString();
  let workspace = await mutateWorkspace(workspaceId, (current) =>
    beginWorkspaceTurn(current, { createdAt, id: turnId, text: message })
  );

  let run;
  try {
    run = await start(workspaceTurnWorkflow, [{ turnId, workspaceId }]);
  } catch (error) {
    const failedAt = new Date().toISOString();
    await mutateWorkspace(workspaceId, (current) =>
      current.activeTurnId === turnId
        ? {
            ...current,
            activeTurnId: undefined,
            error: "Could not start workspace turn.",
            sandbox: { ...current.sandbox, status: "error" },
            status: "error",
            updatedAt: failedAt
          }
        : current
    ).catch(() => {});
    throw error;
  }
  workspace = await mutateWorkspace(workspaceId, (current) =>
    recordWorkspaceWorkflowRun(current, turnId, run.runId)
  );
  return toWorkspaceView(workspace);
}
