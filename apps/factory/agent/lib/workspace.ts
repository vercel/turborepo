export const WORKSPACE_CREATE_ACTION = "create-workspace";
export const WORKSPACE_ACCESS_ACTION = "access-workspace-sandbox";
export const WORKSPACE_TURN_ACTION = "send-workspace-message";
export const WORKSPACE_TERMINAL_ACTION = "open-workspace-terminal";

export type WorkspaceStatus = "idle" | "running" | "error";

export interface WorkspaceMessage {
  readonly createdAt: string;
  readonly id: string;
  readonly role: "user" | "assistant";
  readonly text: string;
}

export interface WorkspaceRecord {
  readonly activeDispatchId?: string;
  readonly activeTurnId?: string;
  readonly createdAt: string;
  readonly publishToken?: string;
  readonly error?: string;
  readonly agent: "fx";
  readonly id: string;
  readonly messages: readonly WorkspaceMessage[];
  readonly pullRequest?: { readonly number: number; readonly url: string };
  readonly sandbox: {
    readonly name: string;
    readonly provider: "vercel";
    readonly status: "pending" | "running" | "error";
  };
  readonly sessionId?: string;
  readonly status: WorkspaceStatus;
  readonly title: string;
  readonly updatedAt: string;
  readonly version: 1;
  readonly workflowRunId?: string;
}

export type WorkspaceView = Omit<
  WorkspaceRecord,
  "activeDispatchId" | "activeTurnId" | "publishToken"
>;

export interface PublicWorkspaceView extends WorkspaceView {
  readonly chatCommand?: string;
}

export type WorkspaceSummary = Pick<
  WorkspaceRecord,
  "createdAt" | "id" | "status" | "title" | "updatedAt"
>;

const ID_PATTERN = /^[A-Za-z0-9_-]{1,128}$/;
const MAX_MESSAGES = 1000;
const STALE_TURN_MS = 60 * 60 * 1000;

export function workspaceSandboxName(workspaceId: string): string {
  return `factory-workspace-${workspaceId}`;
}

export function isWorkspaceId(value: unknown): value is string {
  return typeof value === "string" && /^ws_[A-Za-z0-9_-]{1,96}$/.test(value);
}

export function parseCreateWorkspaceInput(value: unknown): {
  readonly prompt?: string;
  readonly title: string;
} | null {
  if (!isObject(value)) return null;
  if (value.title !== undefined && typeof value.title !== "string") return null;
  if (value.prompt !== undefined && typeof value.prompt !== "string")
    return null;
  const prompt = value.prompt?.trim();
  const title = value.title?.trim() || prompt?.split("\n", 1)[0]?.slice(0, 120);
  if (
    !title ||
    title.length > 120 ||
    (prompt !== undefined && (prompt.length === 0 || prompt.length > 20_000))
  )
    return null;
  return {
    ...(prompt === undefined ? {} : { prompt }),
    title
  };
}

export function parseWorkspaceTurnInput(
  value: unknown
): { readonly message: string } | null {
  if (!isObject(value) || typeof value.message !== "string") return null;
  const message = value.message.trim();
  return message.length > 0 && message.length <= 20_000 ? { message } : null;
}

export function isWorkspaceMutationRequest(
  request: Request,
  action: string,
  options?: { readonly requireJson?: boolean }
): boolean {
  return (
    request.headers.get("origin") === new URL(request.url).origin &&
    (options?.requireJson === false ||
      request.headers.get("content-type")?.split(";", 1)[0] ===
        "application/json") &&
    request.headers.get("x-operator-action") === action
  );
}

export function beginWorkspaceTurn(
  workspace: WorkspaceRecord,
  input: {
    readonly createdAt: string;
    readonly id: string;
    readonly text: string;
  }
): WorkspaceRecord | null {
  const stale =
    workspace.status === "running" &&
    workspace.workflowRunId === undefined &&
    Date.parse(input.createdAt) - Date.parse(workspace.updatedAt) >=
      STALE_TURN_MS;
  if (workspace.status === "running" && !stale) return null;
  return {
    ...workspace,
    activeDispatchId: undefined,
    activeTurnId: input.id,
    error: undefined,
    messages: [
      ...workspace.messages,
      { ...input, role: "user" as const }
    ].slice(-MAX_MESSAGES),
    sandbox: { ...workspace.sandbox, status: "running" },
    status: "running",
    updatedAt: input.createdAt,
    workflowRunId: undefined
  };
}

export function recordWorkspaceWorkflowRun(
  workspace: WorkspaceRecord,
  turnId: string,
  workflowRunId: string
): WorkspaceRecord {
  const completedThisTurn =
    workspace.activeTurnId === undefined &&
    workspace.messages.at(-1)?.id === `msg_${turnId}`;
  return workspace.activeTurnId === turnId || completedThisTurn
    ? { ...workspace, workflowRunId }
    : workspace;
}

export function recoverTerminalWorkspaceTurn(
  workspace: WorkspaceRecord,
  expectedWorkflowRunId: string,
  workflowStatus: string,
  updatedAt: string
): WorkspaceRecord {
  if (
    workspace.status !== "running" ||
    workspace.workflowRunId !== expectedWorkflowRunId ||
    !["cancelled", "completed", "failed"].includes(workflowStatus)
  )
    return workspace;
  return {
    ...workspace,
    activeDispatchId: undefined,
    activeTurnId: undefined,
    error:
      "The Workflow ended before it finalized this workspace. Inspect its audit and sandbox before continuing.",
    sandbox: { ...workspace.sandbox, status: "running" },
    status: "error",
    updatedAt
  };
}

export function toWorkspaceView(
  workspace: WorkspaceRecord
): PublicWorkspaceView {
  const chatCommand = workspaceChatCommand(workspace);
  return {
    ...(chatCommand === undefined ? {} : { chatCommand }),
    createdAt: workspace.createdAt,
    ...(workspace.error === undefined ? {} : { error: workspace.error }),
    agent: workspace.agent,
    id: workspace.id,
    messages: workspace.messages,
    ...(workspace.pullRequest === undefined
      ? {}
      : { pullRequest: workspace.pullRequest }),
    sandbox: workspace.sandbox,
    sessionId: workspace.sessionId,
    status: workspace.status,
    title: workspace.title,
    updatedAt: workspace.updatedAt,
    version: workspace.version,
    ...(workspace.workflowRunId === undefined
      ? {}
      : { workflowRunId: workspace.workflowRunId })
  };
}

export function toWorkspaceSummary(
  workspace: WorkspaceRecord
): WorkspaceSummary {
  return {
    createdAt: workspace.createdAt,
    id: workspace.id,
    status: workspace.status,
    title: workspace.title,
    updatedAt: workspace.updatedAt
  };
}

export function isSafeWorkspaceDiffPath(path: string): boolean {
  return !path
    .split("/")
    .some((part) =>
      /^(?:\.env(?:\..*)?|\.npmrc|\.netrc|\.pypirc|.*\.(?:key|pem))$/i.test(
        part
      )
    );
}

function workspaceChatCommand(workspace: WorkspaceRecord): string | undefined {
  return workspace.sessionId
    ? `fx resume --id ${workspace.sessionId}`
    : undefined;
}

export function isWorkspaceRecord(value: unknown): value is WorkspaceRecord {
  if (!isObject(value) || !Array.isArray(value.messages)) return false;
  const sandbox = value.sandbox;
  return (
    value.version === 1 &&
    isWorkspaceId(value.id) &&
    typeof value.title === "string" &&
    value.title.length > 0 &&
    value.title.length <= 120 &&
    (value.status === "idle" ||
      value.status === "running" ||
      value.status === "error") &&
    value.agent === "fx" &&
    (value.sessionId === undefined ||
      (typeof value.sessionId === "string" &&
        ID_PATTERN.test(value.sessionId))) &&
    isObject(sandbox) &&
    sandbox.provider === "vercel" &&
    sandbox.name === workspaceSandboxName(value.id) &&
    (sandbox.status === "pending" ||
      sandbox.status === "running" ||
      sandbox.status === "error") &&
    value.messages.length <= MAX_MESSAGES &&
    value.messages.every(isWorkspaceMessage) &&
    isIsoDate(value.createdAt) &&
    isIsoDate(value.updatedAt) &&
    optionalString(value.workflowRunId, 256) &&
    optionalString(value.activeDispatchId, 128) &&
    optionalString(value.activeTurnId, 128) &&
    optionalString(value.publishToken, 128) &&
    optionalString(value.error, 2000) &&
    (value.pullRequest === undefined || isPullRequest(value.pullRequest))
  );
}

function isWorkspaceMessage(value: unknown): value is WorkspaceMessage {
  return (
    isObject(value) &&
    typeof value.id === "string" &&
    ID_PATTERN.test(value.id) &&
    (value.role === "user" || value.role === "assistant") &&
    typeof value.text === "string" &&
    value.text.length <= 100_000 &&
    isIsoDate(value.createdAt)
  );
}

function isPullRequest(value: unknown): boolean {
  return (
    isObject(value) &&
    Number.isSafeInteger(value.number) &&
    (value.number as number) > 0 &&
    typeof value.url === "string" &&
    value.url.startsWith("https://github.com/vercel/turborepo/pull/")
  );
}

function isIsoDate(value: unknown): value is string {
  return (
    typeof value === "string" &&
    Number.isFinite(Date.parse(value)) &&
    new Date(value).toISOString() === value
  );
}

function optionalString(value: unknown, maxLength: number): boolean {
  return (
    value === undefined ||
    (typeof value === "string" && value.length <= maxLength)
  );
}

function isObject(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}
