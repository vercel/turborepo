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
  readonly url?: string;
  readonly number?: number;
}

export interface PublicWorkspace {
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
