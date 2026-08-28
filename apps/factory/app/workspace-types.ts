export interface WorkspaceMessage {
  readonly id: string;
  readonly role: "user" | "assistant" | "system";
  readonly text: string;
  readonly createdAt: string;
}

export interface WorkspaceSandbox {
  readonly name: string;
  readonly status: string;
}

export interface WorkspacePullRequest {
  readonly checkedAt?: string;
  readonly url?: string;
  readonly number?: number;
  readonly state?: "open" | "closed" | "merged";
}

export interface PublicWorkspace {
  readonly activity?: string;
  readonly chatCommand?: string;
  readonly id: string;
  readonly title: string;
  readonly status: string;
  readonly agent: "fx";
  readonly sandbox: WorkspaceSandbox;
  readonly sessionId?: string;
  readonly messages: readonly WorkspaceMessage[];
  readonly createdAt: string;
  readonly updatedAt: string;
  readonly error?: string;
  readonly workflowRunId?: string;
  readonly pullRequest?: string | WorkspacePullRequest;
}

export interface WorkspaceSummary {
  readonly activity?: string;
  readonly id: string;
  readonly title: string;
  readonly status: string;
  readonly pullRequest?: WorkspacePullRequest;
  readonly createdAt: string;
  readonly updatedAt: string;
}

export function isWorkspaceRunning(status: string): boolean {
  return ["creating", "pending", "queued", "starting", "running"].includes(
    status
  );
}

export function workspaceStatusLabel(
  workspace: Pick<WorkspaceSummary, "activity" | "pullRequest" | "status">
): string {
  if (isWorkspaceRunning(workspace.status))
    return workspace.activity ?? "Working";
  if (workspace.status === "done") return "Done";
  if (workspace.status === "error") return "Error";
  if (workspace.pullRequest?.state === "closed") return "PR closed";
  if (workspace.pullRequest) return "PR open";
  if (workspace.status === "idle") return "Ready";
  return workspace.status;
}
