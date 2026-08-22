import { randomUUID } from "node:crypto";

import { getRun, start } from "workflow/api";

import { workspaceTurnWorkflow } from "../../workflows/workspace-turn";
import {
  beginWorkspaceTurn,
  recoverTerminalWorkspaceTurn,
  recordWorkspaceWorkflowRun,
  toWorkspaceView,
  type PublicWorkspaceView
} from "./workspace";
import { getWorkspace, mutateWorkspace } from "./workspace-store";

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
