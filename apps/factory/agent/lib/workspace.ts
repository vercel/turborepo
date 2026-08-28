export const WORKSPACE_CREATE_ACTION = "create-workspace";
export const WORKSPACE_TERMINAL_ACTION = "open-workspace-terminal";

export type WorkspaceStatus = "idle" | "running" | "error";

export const DEFAULT_WORKSPACE_MODEL = "openai/gpt-5.6-sol";

export interface WorkspaceMessage {
  readonly createdAt: string;
  readonly id: string;
  readonly role: "user" | "assistant";
  readonly text: string;
}

export interface WorkspaceRecord {
  readonly activeTurnId?: string;
  readonly createdAt: string;
  readonly error?: string;
  readonly agent: "eve";
  readonly id: string;
  readonly messages: readonly WorkspaceMessage[];
  readonly model?: string;
  readonly pullRequest?: { readonly number: number; readonly url: string };
  readonly sandbox: {
    readonly id?: string;
    readonly provider: "vercel";
    readonly status: "pending" | "running" | "error";
  };
  readonly sessionId?: string;
  readonly status: WorkspaceStatus;
  readonly title: string;
  readonly updatedAt: string;
  readonly version: 2;
}

export type WorkspaceView = Omit<WorkspaceRecord, "activeTurnId">;
export type PublicWorkspaceView = WorkspaceView;

export type WorkspaceSummary = Pick<
  WorkspaceRecord,
  "createdAt" | "id" | "status" | "title" | "updatedAt"
>;

const ID_PATTERN = /^[A-Za-z0-9_-]{1,128}$/;
const MAX_MESSAGES = 1000;

export const WORKSPACE_RUN_MODE = "conversation" as const;

export function isWorkspaceId(value: unknown): value is string {
  return typeof value === "string" && /^ws_[A-Za-z0-9_-]{1,96}$/.test(value);
}

export function parseCreateWorkspaceInput(value: unknown): {
  readonly model: string;
  readonly prompt?: string;
  readonly title: string;
} | null {
  if (!isObject(value)) return null;
  if (value.title !== undefined && typeof value.title !== "string") return null;
  if (value.prompt !== undefined && typeof value.prompt !== "string")
    return null;
  if (value.model !== undefined && !isWorkspaceModel(value.model)) return null;
  const prompt = value.prompt?.trim();
  const title = value.title?.trim() || prompt?.split("\n", 1)[0]?.slice(0, 120);
  if (
    !title ||
    title.length > 120 ||
    (prompt !== undefined && (prompt.length === 0 || prompt.length > 20_000))
  )
    return null;
  return {
    model: value.model ?? DEFAULT_WORKSPACE_MODEL,
    ...(prompt === undefined ? {} : { prompt }),
    title
  };
}

export function isWorkspaceMutationRequest(
  request: Request,
  action: string
): boolean {
  const origin = request.headers.get("origin");
  const host =
    request.headers.get("x-forwarded-host") ?? request.headers.get("host");
  return (
    origin !== null &&
    host !== null &&
    URL.parse(origin)?.host === host &&
    request.headers.get("sec-fetch-site") !== "cross-site" &&
    request.headers.get("content-type")?.split(";", 1)[0] ===
      "application/json" &&
    request.headers.get("x-operator-action") === action
  );
}

export function toWorkspaceView(
  workspace: WorkspaceRecord
): PublicWorkspaceView {
  return {
    createdAt: workspace.createdAt,
    ...(workspace.error === undefined ? {} : { error: workspace.error }),
    agent: workspace.agent,
    id: workspace.id,
    messages: workspace.messages,
    model: workspace.model ?? DEFAULT_WORKSPACE_MODEL,
    ...(workspace.pullRequest === undefined
      ? {}
      : { pullRequest: workspace.pullRequest }),
    sandbox: workspace.sandbox,
    sessionId: workspace.sessionId,
    status: workspace.status,
    title: workspace.title,
    updatedAt: workspace.updatedAt,
    version: workspace.version
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

export function isWorkspaceRecord(value: unknown): value is WorkspaceRecord {
  if (!isObject(value) || !Array.isArray(value.messages)) return false;
  const sandbox = value.sandbox;
  return (
    value.version === 2 &&
    isWorkspaceId(value.id) &&
    typeof value.title === "string" &&
    value.title.length > 0 &&
    value.title.length <= 120 &&
    (value.status === "idle" ||
      value.status === "running" ||
      value.status === "error") &&
    value.agent === "eve" &&
    (value.model === undefined || isWorkspaceModel(value.model)) &&
    optionalString(value.sessionId, 256) &&
    isObject(sandbox) &&
    sandbox.provider === "vercel" &&
    optionalString(sandbox.id, 256) &&
    (sandbox.status === "pending" ||
      sandbox.status === "running" ||
      sandbox.status === "error") &&
    value.messages.length <= MAX_MESSAGES &&
    value.messages.every(isWorkspaceMessage) &&
    isIsoDate(value.createdAt) &&
    isIsoDate(value.updatedAt) &&
    optionalString(value.activeTurnId, 128) &&
    optionalString(value.error, 2000) &&
    (value.pullRequest === undefined || isPullRequest(value.pullRequest))
  );
}

export function isWorkspaceModel(value: unknown): value is string {
  return (
    typeof value === "string" &&
    value.length <= 200 &&
    /^[A-Za-z0-9][A-Za-z0-9._-]*\/[A-Za-z0-9][A-Za-z0-9._:-]*$/.test(value)
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

function isObject(value: unknown): value is Record<string, any> {
  return typeof value === "object" && value !== null;
}
