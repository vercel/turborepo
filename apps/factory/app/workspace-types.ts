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
  readonly thinkingEffort?: "low" | "medium" | "high";
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
 * boundary independent of Eve's event types keeps failure handling separate
 * from the workspace presentation.
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

export const WORKSPACE_IMAGE_MEDIA_TYPES = [
  "image/png",
  "image/jpeg",
  "image/webp",
  "image/gif"
] as const;
export const MAX_WORKSPACE_IMAGES = 4;
// Base64 plus the JSON envelope stays below Vercel's 4.5 MB request limit.
export const MAX_WORKSPACE_IMAGE_BYTES = 2 * 1024 * 1024;

export interface WorkspaceChatImage {
  readonly id: string;
  readonly name: string;
  readonly mediaType: string;
  readonly size: number;
  readonly url: string;
}

export interface WorkspaceChatDraft {
  readonly text: string;
  readonly images?: readonly WorkspaceChatImage[];
}

type ProjectedChatMessage = {
  readonly role: string;
  readonly parts: readonly {
    readonly type: string;
    readonly text?: string;
    readonly url?: string;
    readonly filename?: string;
    readonly mediaType?: string;
    readonly size?: number;
  }[];
};

export function workspaceImageSelectionError(
  files: readonly Pick<File, "type" | "size">[],
  existing: readonly WorkspaceChatImage[]
): string | undefined {
  if (files.length + existing.length > MAX_WORKSPACE_IMAGES)
    return `Attach at most ${MAX_WORKSPACE_IMAGES} images per message.`;
  if (
    files.some(
      (file) => !WORKSPACE_IMAGE_MEDIA_TYPES.some((type) => type === file.type)
    )
  )
    return "Use PNG, JPEG, WebP, or GIF images.";
  if (files.some((file) => file.size === 0))
    return "Cannot attach an empty image.";
  if (
    files.reduce((size, file) => size + file.size, 0) +
      existing.reduce((size, image) => size + image.size, 0) >
    MAX_WORKSPACE_IMAGE_BYTES
  )
    return "Images must total 2 MiB or less per message.";
  return undefined;
}

export async function readWorkspaceChatImages(
  files: readonly File[],
  existing: readonly WorkspaceChatImage[]
): Promise<WorkspaceChatImage[]> {
  const error = workspaceImageSelectionError(files, existing);
  if (error) throw new Error(error);
  return Promise.all(
    files.map(
      (file) =>
        new Promise<WorkspaceChatImage>((resolve, reject) => {
          const reader = new FileReader();
          reader.onerror = () =>
            reject(new Error(`Could not read ${file.name}.`));
          reader.onabort = () =>
            reject(new Error(`Reading ${file.name} was cancelled.`));
          reader.onload = () => {
            if (
              typeof reader.result !== "string" ||
              !workspaceImageUrl(file.type, reader.result)
            ) {
              reject(new Error(`Could not read ${file.name} as an image.`));
              return;
            }
            resolve({
              id: crypto.randomUUID(),
              name: file.name,
              mediaType: file.type,
              size: file.size,
              url: reader.result
            });
          };
          reader.readAsDataURL(file);
        })
    )
  );
}

/** Eve accepts file data URLs and persists their browser-resolvable URLs in the transcript. */
export function workspaceChatContent(draft: WorkspaceChatDraft) {
  if (!draft.images?.length) return draft.text;
  return [
    ...(draft.text ? [{ type: "text" as const, text: draft.text }] : []),
    ...draft.images.map((image) => ({
      type: "file" as const,
      data: image.url,
      mediaType: image.mediaType,
      filename: image.name
    }))
  ];
}

export function workspaceImageUrl(
  mediaType: string,
  url?: string
): string | undefined {
  if (!url || !WORKSPACE_IMAGE_MEDIA_TYPES.some((type) => type === mediaType))
    return undefined;
  if (url.startsWith(`data:${mediaType};base64,`)) return url;
  try {
    return new URL(url).protocol === "https:" ? url : undefined;
  } catch {
    return undefined;
  }
}

function matchesChatDraft(
  message: ProjectedChatMessage,
  draft: WorkspaceChatDraft
): boolean {
  if (message.role !== "user") return false;
  const text = message.parts
    .filter((part) => part.type === "text")
    .map((part) => part.text)
    .join("\n")
    .trim();
  const files = message.parts.filter((part) => part.type === "file");
  const images = draft.images ?? [];
  return (
    text === draft.text.trim() &&
    files.length === images.length &&
    images.every((image, index) => {
      const file = files[index];
      return (
        file?.mediaType === image.mediaType &&
        (file.url
          ? file.url === image.url
          : file.filename === image.name && file.size === image.size)
      );
    })
  );
}

export function hasConfirmedWorkspaceMessage(
  messages: readonly ProjectedChatMessage[],
  draft: WorkspaceChatDraft & { readonly afterMessageCount: number }
): boolean {
  return messages
    .slice(draft.afterMessageCount)
    .some((message) => matchesChatDraft(message, draft));
}

/** Keep React state referentially unchanged unless a sent message is confirmed. */
export function removeConfirmedOptimisticMessages<
  T extends WorkspaceChatDraft & { readonly afterMessageCount: number }
>(current: T[], messages: readonly ProjectedChatMessage[]): T[] {
  const confirmed = new Set<number>();
  let changed = false;
  const pending: T[] = [];
  for (const sent of current) {
    // Don't reuse a previous acknowledgement for a repeated sent draft,
    // including on the next cleanup after the first entry has been removed.
    let afterMessageCount = sent.afterMessageCount;
    for (const index of confirmed) {
      if (matchesChatDraft(messages[index], sent))
        afterMessageCount = Math.max(afterMessageCount, index + 1);
    }
    const index = messages.findIndex(
      (message, index) =>
        index >= afterMessageCount &&
        !confirmed.has(index) &&
        matchesChatDraft(message, sent)
    );
    if (index !== -1) {
      confirmed.add(index);
      changed = true;
    } else if (afterMessageCount !== sent.afterMessageCount) {
      pending.push({ ...sent, afterMessageCount });
      changed = true;
    } else pending.push(sent);
  }
  return changed ? pending : current;
}

// Steering replaces an active response rather than waiting for it to finish.
export const WORKSPACE_CHAT_TURN_POLICY = "steer" as const;

export function activeWorkspaceTurnId(
  events: readonly WorkspaceEvent[]
): string | undefined {
  let turnId: string | undefined;
  for (const event of events) {
    if (event.type === "session.failed" || event.type === "session.completed") {
      turnId = undefined;
      continue;
    }
    if (!isRecord(event.data)) continue;
    if (event.type === "turn.started" && typeof event.data.turnId === "string")
      turnId = event.data.turnId;
    else if (
      ["turn.completed", "turn.cancelled", "turn.failed"].includes(
        event.type
      ) &&
      event.data.turnId === turnId
    )
      turnId = undefined;
  }
  return turnId;
}
