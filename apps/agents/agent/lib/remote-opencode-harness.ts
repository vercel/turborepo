import { createHash } from "node:crypto";

import {
  HarnessCapabilityUnsupportedError,
  type HarnessV1,
  type HarnessV1ContinueTurnState,
  type HarnessV1Prompt,
  type HarnessV1PromptTurnOptions,
  type HarnessV1ResumeSessionState,
  type HarnessV1Session,
  type HarnessV1StreamPart
} from "@ai-sdk/harness";

const HARNESS_ID = "remote-opencode";

interface OpenCodeLocation {
  readonly directory: string;
}

interface OpenCodeModel {
  readonly id: string;
  readonly providerID: string;
  readonly variant?: string;
}

interface OpenCodeSession {
  readonly agent?: string;
  readonly id: string;
  readonly location: OpenCodeLocation;
  readonly model?: OpenCodeModel;
  readonly title?: string;
}

interface OpenCodeEvent {
  readonly data?: Record<string, unknown>;
  readonly durable?: { readonly seq?: number };
  readonly type?: string;
}

interface RemoteState {
  readonly cursor: number;
  readonly openCodeSessionID: string;
  readonly turn: number;
}

export interface RemoteOpenCodeHarnessSettings {
  readonly agent?: string;
  readonly baseURL: string;
  readonly fetch?: typeof globalThis.fetch;
  readonly headers?: HeadersInit | (() => HeadersInit | Promise<HeadersInit>);
  readonly location: OpenCodeLocation;
  readonly model?: OpenCodeModel;
  readonly title?: string | ((sessionID: string) => string);
}

function stableID(prefix: string, ...values: readonly string[]): string {
  return `${prefix}_${createHash("sha256").update(values.join("\0")).digest("hex").slice(0, 32)}`;
}

export function openCodeSessionID(sessionID: string): string {
  return sessionID.startsWith("ses")
    ? sessionID
    : stableID("ses_harness", sessionID);
}

function promptText(prompt: HarnessV1Prompt): string {
  if (typeof prompt === "string") return prompt;
  if (typeof prompt.content === "string") return prompt.content;

  if (prompt.content.some((part) => part.type !== "text")) {
    throw new HarnessCapabilityUnsupportedError({
      harnessId: HARNESS_ID,
      message:
        "The remote OpenCode harness currently accepts text prompts only."
    });
  }

  const text = prompt.content
    .filter(
      (
        part
      ): part is Extract<(typeof prompt.content)[number], { type: "text" }> =>
        part.type === "text"
    )
    .map((part) => part.text)
    .join("\n");
  if (!text) {
    throw new HarnessCapabilityUnsupportedError({
      harnessId: HARNESS_ID,
      message:
        "The remote OpenCode harness currently accepts text prompts only."
    });
  }
  return text;
}

function lifecycleState(state: RemoteState): HarnessV1ResumeSessionState {
  return {
    data: {
      cursor: state.cursor,
      openCodeSessionID: state.openCodeSessionID,
      turn: state.turn
    },
    harnessId: HARNESS_ID,
    specificationVersion: "harness-v1",
    type: "resume-session"
  };
}

function readState(
  value: HarnessV1ResumeSessionState | HarnessV1ContinueTurnState | undefined
): RemoteState | undefined {
  if (!value) return undefined;
  if (
    value.harnessId !== HARNESS_ID ||
    value.specificationVersion !== "harness-v1" ||
    typeof value.data !== "object" ||
    value.data === null
  )
    throw new Error("Invalid remote OpenCode lifecycle state.");
  const state = value.data as Record<string, unknown>;
  if (
    !(
      Number.isInteger(state.cursor) &&
      Number(state.cursor) >= 0 &&
      typeof state.openCodeSessionID === "string" &&
      Number.isInteger(state.turn) &&
      Number(state.turn) >= 0
    )
  ) {
    throw new Error("Invalid remote OpenCode lifecycle state.");
  }
  return {
    cursor: Number(state.cursor),
    openCodeSessionID: state.openCodeSessionID,
    turn: Number(state.turn)
  };
}

function unwrapSession(value: unknown): OpenCodeSession {
  const candidate =
    typeof value === "object" && value !== null && "data" in value
      ? (value as { data: unknown }).data
      : value;
  if (
    typeof candidate !== "object" ||
    candidate === null ||
    !("id" in candidate) ||
    typeof candidate.id !== "string" ||
    !("location" in candidate) ||
    typeof candidate.location !== "object" ||
    candidate.location === null ||
    !("directory" in candidate.location) ||
    typeof candidate.location.directory !== "string"
  ) {
    throw new Error("OpenCode returned an invalid session.");
  }
  return candidate as unknown as OpenCodeSession;
}

function assertSession(
  session: OpenCodeSession,
  expected: OpenCodeSession
): void {
  const mismatches = [
    session.id === expected.id ? null : "id",
    session.location.directory === expected.location.directory
      ? null
      : "location",
    expected.title === undefined || session.title === expected.title
      ? null
      : "title",
    expected.agent === undefined || session.agent === expected.agent
      ? null
      : "agent",
    expected.model === undefined ||
    (session.model?.providerID === expected.model.providerID &&
      session.model.id === expected.model.id &&
      session.model.variant === expected.model.variant)
      ? null
      : "model"
  ].filter(Boolean);
  if (mismatches.length > 0) {
    throw new Error(
      `Existing OpenCode session does not match requested ${mismatches.join(", ")}.`
    );
  }
}

function eventSequence(event: OpenCodeEvent): number {
  return event.durable?.seq ?? 0;
}

function usage(tokens: unknown) {
  const value =
    typeof tokens === "object" && tokens !== null
      ? (tokens as Record<string, unknown>)
      : {};
  const cache =
    typeof value.cache === "object" && value.cache !== null
      ? (value.cache as Record<string, unknown>)
      : {};
  const input = typeof value.input === "number" ? value.input : 0;
  const output = typeof value.output === "number" ? value.output : 0;
  const reasoning = typeof value.reasoning === "number" ? value.reasoning : 0;
  const cacheRead = typeof cache.read === "number" ? cache.read : 0;
  const cacheWrite = typeof cache.write === "number" ? cache.write : 0;
  return {
    inputTokens: {
      cacheRead,
      cacheWrite,
      noCache: Math.max(0, input - cacheRead - cacheWrite),
      total: input
    },
    outputTokens: {
      reasoning,
      text: Math.max(0, output - reasoning),
      total: output
    }
  };
}

function addUsage(
  left: ReturnType<typeof usage>,
  right: ReturnType<typeof usage>
): ReturnType<typeof usage> {
  return {
    inputTokens: {
      cacheRead:
        (left.inputTokens.cacheRead ?? 0) + (right.inputTokens.cacheRead ?? 0),
      cacheWrite:
        (left.inputTokens.cacheWrite ?? 0) +
        (right.inputTokens.cacheWrite ?? 0),
      noCache:
        (left.inputTokens.noCache ?? 0) + (right.inputTokens.noCache ?? 0),
      total: (left.inputTokens.total ?? 0) + (right.inputTokens.total ?? 0)
    },
    outputTokens: {
      reasoning:
        (left.outputTokens.reasoning ?? 0) +
        (right.outputTokens.reasoning ?? 0),
      text: (left.outputTokens.text ?? 0) + (right.outputTokens.text ?? 0),
      total: (left.outputTokens.total ?? 0) + (right.outputTokens.total ?? 0)
    }
  };
}

function finishReason(value: unknown) {
  const raw = typeof value === "string" ? value : "unknown";
  return {
    raw,
    unified:
      raw === "stop" ||
      raw === "length" ||
      raw === "content-filter" ||
      raw === "tool-calls" ||
      raw === "error"
        ? raw
        : "other"
  } as const;
}

function emitEvents(
  events: readonly OpenCodeEvent[],
  emit: (event: HarnessV1StreamPart) => void
): void {
  emit({ type: "stream-start" });
  let totalUsage = usage(undefined);
  let finalReason = finishReason("stop");

  for (const event of events) {
    const data = event.data ?? {};
    if (event.type === "session.text.ended" && typeof data.text === "string") {
      const id = `${String(data.assistantMessageID ?? "text")}:${String(data.ordinal ?? 0)}`;
      emit({ type: "text-start", id });
      emit({ type: "text-delta", id, delta: data.text });
      emit({ type: "text-end", id });
    }
    if (
      event.type === "session.reasoning.ended" &&
      typeof data.text === "string"
    ) {
      const id = `${String(data.assistantMessageID ?? "reasoning")}:${String(data.ordinal ?? 0)}`;
      emit({ type: "reasoning-start", id });
      emit({ type: "reasoning-delta", id, delta: data.text });
      emit({ type: "reasoning-end", id });
    }
    if (event.type === "session.step.ended") {
      const stepUsage = usage(data.tokens);
      const reason = finishReason(data.finish);
      totalUsage = addUsage(totalUsage, stepUsage);
      finalReason = reason;
      emit({ type: "finish-step", finishReason: reason, usage: stepUsage });
    }
  }

  emit({ type: "finish", finishReason: finalReason, totalUsage });
}

async function* parseServerEvents(
  response: Response
): AsyncGenerator<OpenCodeEvent> {
  if (!response.body) return;
  const reader = response.body.getReader();
  const decoder = new TextDecoder();
  let buffer = "";

  while (true) {
    const { done, value } = await reader.read();
    buffer += decoder.decode(value, { stream: !done });
    const blocks = buffer.split(/\r?\n\r?\n/);
    buffer = blocks.pop() ?? "";
    for (const block of blocks) {
      const data = block
        .split(/\r?\n/)
        .filter((line) => line.startsWith("data:"))
        .map((line) => line.slice(5).trimStart())
        .join("\n");
      if (data && data !== "[DONE]") yield JSON.parse(data) as OpenCodeEvent;
    }
    if (done) {
      const data = buffer
        .split(/\r?\n/)
        .filter((line) => line.startsWith("data:"))
        .map((line) => line.slice(5).trimStart())
        .join("\n");
      if (data && data !== "[DONE]") yield JSON.parse(data) as OpenCodeEvent;
      break;
    }
  }
}

export function createRemoteOpenCode(
  settings: RemoteOpenCodeHarnessSettings
): HarnessV1 {
  const fetcher = settings.fetch ?? globalThis.fetch;
  const baseURL = settings.baseURL.replace(/\/$/, "");
  if (new URL(baseURL).protocol !== "https:" && settings.fetch === undefined) {
    throw new Error("Remote OpenCode requires an HTTPS server URL.");
  }

  async function request(
    path: string,
    init: RequestInit = {},
    allowed = [200, 204]
  ): Promise<Response> {
    const configuredHeaders =
      typeof settings.headers === "function"
        ? await settings.headers()
        : settings.headers;
    const headers = new Headers(configuredHeaders);
    new Headers(init.headers).forEach((value, key) => headers.set(key, value));
    if (init.body) headers.set("content-type", "application/json");
    const response = await fetcher(`${baseURL}${path}`, {
      ...init,
      headers,
      redirect: "error"
    });
    if (!allowed.includes(response.status)) {
      const detail = (await response.text()).slice(0, 500);
      throw new Error(
        `OpenCode ${init.method ?? "GET"} ${path} failed (${response.status}): ${detail}`
      );
    }
    return response;
  }

  async function eventLog(
    sessionID: string,
    after: number,
    signal?: AbortSignal
  ): Promise<OpenCodeEvent[]> {
    const query = new URLSearchParams({
      after: String(after),
      follow: "false"
    });
    const response = await request(
      `/api/experimental/session/${encodeURIComponent(sessionID)}/log?${query}`,
      { headers: { accept: "text/event-stream" }, signal }
    );
    const events: OpenCodeEvent[] = [];
    for await (const event of parseServerEvents(response)) events.push(event);
    return events;
  }

  return {
    builtinTools: {},
    harnessId: HARNESS_ID,
    specificationVersion: "harness-v1",
    async doStart(options) {
      const id = openCodeSessionID(options.sessionId);
      const title =
        typeof settings.title === "function"
          ? settings.title(options.sessionId)
          : settings.title;
      const expected: OpenCodeSession = {
        agent: settings.agent,
        id,
        location: settings.location,
        model: settings.model,
        title
      };
      const existing = await request(
        `/api/session/${encodeURIComponent(id)}`,
        { signal: options.abortSignal },
        [200, 404]
      );
      const session =
        existing.status === 404
          ? unwrapSession(
              await (
                await request("/api/session", {
                  body: JSON.stringify(expected),
                  method: "POST",
                  signal: options.abortSignal
                })
              ).json()
            )
          : unwrapSession(await existing.json());
      assertSession(session, expected);

      const resumed = readState(options.continueFrom ?? options.resumeFrom);
      let state: RemoteState = resumed ?? {
        cursor: 0,
        openCodeSessionID: id,
        turn: 0
      };
      if (state.openCodeSessionID !== id) {
        throw new Error(
          "Remote OpenCode lifecycle state belongs to another session."
        );
      }

      const remoteSession: HarnessV1Session = {
        isResume: resumed !== undefined,
        sessionId: options.sessionId,
        async doPromptTurn(turnOptions: HarnessV1PromptTurnOptions) {
          if (turnOptions.tools && turnOptions.tools.length > 0) {
            throw new HarnessCapabilityUnsupportedError({
              harnessId: HARNESS_ID,
              message:
                "Host-executed Harness tools are not supported; register tools with OpenCode instead."
            });
          }
          const input = [
            state.turn === 0 ? turnOptions.instructions : undefined,
            promptText(turnOptions.prompt)
          ]
            .filter(Boolean)
            .join("\n\n");
          const promptID = stableID(
            "msg_harness",
            id,
            String(state.turn),
            input
          );
          const done = (async () => {
            try {
              let events = await eventLog(
                id,
                state.cursor,
                turnOptions.abortSignal
              );
              let promptEvent = events.find(
                (event) =>
                  event.type === "session.inbox.enqueued" &&
                  event.data?.inboxID === promptID
              );
              if (!promptEvent) {
                await request(`/api/session/${encodeURIComponent(id)}/prompt`, {
                  body: JSON.stringify({
                    delivery: "queue",
                    id: promptID,
                    metadata: {
                      origin: "harness",
                      harnessSessionID: options.sessionId
                    },
                    resume: true,
                    text: input
                  }),
                  method: "POST",
                  signal: turnOptions.abortSignal
                });
              }
              await request(`/api/session/${encodeURIComponent(id)}/wait`, {
                method: "POST",
                signal: turnOptions.abortSignal
              });
              events = await eventLog(
                id,
                state.cursor,
                turnOptions.abortSignal
              );
              promptEvent = events.find(
                (event) =>
                  event.type === "session.inbox.enqueued" &&
                  event.data?.inboxID === promptID
              );
              if (!promptEvent)
                throw new Error(
                  "OpenCode completed without recording the submitted prompt."
                );
              const promptSequence = eventSequence(promptEvent);
              const turnEvents = events.filter(
                (event) => eventSequence(event) >= promptSequence
              );
              const failure = turnEvents.find(
                (event) => event.type === "session.execution.failed"
              );
              if (failure)
                throw new Error(
                  `OpenCode execution failed: ${JSON.stringify(failure.data?.error)}`
                );
              if (
                !turnEvents.some(
                  (event) => event.type === "session.execution.succeeded"
                )
              ) {
                throw new Error(
                  "OpenCode execution ended without a success event."
                );
              }
              emitEvents(turnEvents, turnOptions.emit);
              state = {
                cursor: events.reduce(
                  (max, event) => Math.max(max, eventSequence(event)),
                  state.cursor
                ),
                openCodeSessionID: id,
                turn: state.turn + 1
              };
            } catch (error) {
              if (turnOptions.abortSignal?.aborted) {
                void request(
                  `/api/session/${encodeURIComponent(id)}/interrupt`,
                  { method: "POST" }
                ).catch(() => {});
              }
              turnOptions.emit({ type: "error", error });
              throw error;
            }
          })();
          return {
            done,
            submitToolResult: async () => {
              throw new HarnessCapabilityUnsupportedError({
                harnessId: HARNESS_ID,
                message: "Remote OpenCode executes tools inside OpenCode."
              });
            }
          };
        },
        async doCompact() {
          await request(`/api/session/${encodeURIComponent(id)}/compact`, {
            body: JSON.stringify({
              id: stableID("msg_compact", id, String(state.turn))
            }),
            method: "POST"
          });
          await request(`/api/session/${encodeURIComponent(id)}/wait`, {
            method: "POST"
          });
        },
        async doContinueTurn() {
          throw new HarnessCapabilityUnsupportedError({
            harnessId: HARNESS_ID,
            message:
              "Suspended-turn continuation is not implemented for remote OpenCode."
          });
        },
        async doSuspendTurn() {
          throw new HarnessCapabilityUnsupportedError({
            harnessId: HARNESS_ID,
            message: "Turn suspension is not implemented for remote OpenCode."
          });
        },
        async doDetach() {
          return lifecycleState(state);
        },
        async doStop() {
          return lifecycleState(state);
        },
        async doDestroy() {
          await request(`/api/session/${encodeURIComponent(id)}`, {
            method: "DELETE"
          });
        }
      };
      return remoteSession;
    }
  };
}
