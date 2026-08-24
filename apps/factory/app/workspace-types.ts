export interface WorkspaceMessage {
  readonly id: string;
  readonly role: "user" | "assistant";
  readonly text: string;
  readonly createdAt: string;
}

export interface WorkspaceSandbox {
  readonly id?: string;
  readonly status: string;
}

export interface WorkspacePullRequest {
  readonly url?: string;
  readonly number?: number;
}

export interface PublicWorkspace {
  readonly id: string;
  readonly title: string;
  readonly status: string;
  readonly agent: "eve";
  readonly sandbox: WorkspaceSandbox;
  readonly sessionId?: string;
  readonly messages: readonly WorkspaceMessage[];
  readonly createdAt: string;
  readonly updatedAt: string;
  readonly error?: string;
  readonly pullRequest?: string | WorkspacePullRequest;
}

export interface WorkspaceSummary {
  readonly id: string;
  readonly title: string;
  readonly status: string;
  readonly createdAt: string;
  readonly updatedAt: string;
}

export function isWorkspaceRunning(status: string): boolean {
  return ["creating", "pending", "queued", "starting", "running"].includes(
    status
  );
}

export function workspaceStatusLabel(status: string): string {
  if (status === "idle") return "Ready";
  if (isWorkspaceRunning(status)) return "Working";
  if (status === "error") return "Error";
  return status;
}
