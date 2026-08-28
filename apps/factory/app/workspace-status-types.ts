import type { WorkspaceSummary as BaseWorkspaceSummary } from "./workspace-types";

export interface WorkspacePullRequestStatus {
  readonly checkedAt?: string;
  readonly number: number;
  readonly state?: "open" | "closed" | "merged";
  readonly url: string;
}

export interface WorkspaceDisplayStatus {
  readonly activity?: string;
  readonly pullRequest?: WorkspacePullRequestStatus;
  readonly status: string;
}

export type WorkspaceSummary = BaseWorkspaceSummary & WorkspaceDisplayStatus;

export function isWorkspaceRunning(status: string): boolean {
  return ["creating", "pending", "queued", "starting", "running"].includes(
    status
  );
}

export function workspaceStatusLabel(workspace: WorkspaceDisplayStatus): string {
  if (isWorkspaceRunning(workspace.status))
    return workspace.activity ?? "Working";
  if (workspace.status === "done") return "Done";
  if (workspace.status === "error") return "Error";
  if (workspace.pullRequest?.state === "closed") return "PR closed";
  if (workspace.pullRequest) return "PR open";
  return workspace.status === "idle" ? "Ready" : workspace.status;
}
