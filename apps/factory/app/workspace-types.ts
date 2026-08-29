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
  readonly model?: string;
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

export interface WorkspaceFailure {
  readonly code?: string;
  readonly detail?: string;
  readonly hint?: string;
  readonly message: string;
}

type WorkspaceEvent = {
  readonly data?: unknown;
  readonly type: string;
};

const FAILURE_EVENTS = new Set([
  "step.failed",
  "turn.failed",
  "session.failed"
]);
const FAILURE_RESET_EVENTS = new Set([
  "turn.started",
  "turn.completed",
  "turn.cancelled",
  "session.completed"
]);

/**
 * Projects the current run failure from an agent event stream. Keeping this
 * boundary independent of Eve's event types lets another harness map its
 * failures into the workspace UI without changing the presentation.
 */
export function latestWorkspaceFailure(
  events: readonly WorkspaceEvent[]
): WorkspaceFailure | undefined {
  let failure: WorkspaceFailure | undefined;
  for (const event of events) {
    if (FAILURE_RESET_EVENTS.has(event.type)) {
      failure = undefined;
      continue;
    }
    if (!FAILURE_EVENTS.has(event.type)) continue;
    const candidate = workspaceFailureFrom(event.data);
    if (candidate) failure = candidate;
  }
  return failure;
}

function workspaceFailureFrom(data: unknown): WorkspaceFailure | undefined {
  if (!isRecord(data) || typeof data.message !== "string") return undefined;
  const message = data.message.trim();
  if (!message) return undefined;
  const details = isRecord(data.details) ? data.details : undefined;
  const code = nonEmptyString(data.code);
  const hint = nonEmptyString(details?.hint);
  const rawDetail = nonEmptyString(details?.detail);
  return {
    ...(code ? { code } : {}),
    ...(rawDetail && rawDetail !== message ? { detail: rawDetail } : {}),
    ...(hint ? { hint } : {}),
    message
  };
}

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}

function nonEmptyString(value: unknown): string | undefined {
  if (typeof value !== "string") return undefined;
  const normalized = value.trim();
  return normalized || undefined;
}
