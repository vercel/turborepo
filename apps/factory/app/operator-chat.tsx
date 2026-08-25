"use client";

import type {
  EveMessage,
  EveMessageInputRequest,
  EveMessagePart
} from "eve/react";
import { useEveAgent } from "eve/react";
import { useEffect, useRef, useState } from "react";

import {
  parseSavedChat,
  type SavedChat,
  serializeSavedChat
} from "../agent/lib/operator-chat-session";
import {
  OPERATOR_ACTION_HEADER,
  OPERATOR_CHAT_ACTION,
  OPERATOR_MODEL_HEADER
} from "../agent/lib/operator-console";
import { GPT_SOL_MODEL } from "../agent/lib/performance-models";
import { Button } from "../components/ui/button";

/**
 * Ad-hoc chat with the factory agent.
 *
 * The scheduled operations start a fixed prompt and report a status. This
 * starts an ordinary durable session instead: the operator types, the agent
 * works in the same sandbox the schedules use, and a pull request happens only
 * when the operator approves the `create_pull_request` call this UI surfaces.
 */

const STORAGE_KEY = "turborepo-factory-operator-chat";
const MODEL_STORAGE_KEY = "turborepo-factory-operator-model";
// Marks every eve request as coming from this page; the eve channel's
// operator-console auth entry requires it. See `agent/lib/operator-console.ts`.
interface AvailableModel {
  readonly id: string;
  readonly name: string;
  readonly ownedBy: string;
}

function consoleHeaders(model: string) {
  return {
    [OPERATOR_ACTION_HEADER]: OPERATOR_CHAT_ACTION,
    [OPERATOR_MODEL_HEADER]: model
  };
}

interface PendingRequest {
  readonly request: EveMessageInputRequest;
  readonly toolName: string;
}

interface OperatorChatProps {
  readonly agentRunsUrl: string;
}

function readSavedChat(): SavedChat | null {
  try {
    return parseSavedChat(window.localStorage.getItem(STORAGE_KEY));
  } catch {
    return null;
  }
}

function writeSavedChat(snapshot: Parameters<typeof serializeSavedChat>[0]) {
  const saved = serializeSavedChat(snapshot);
  if (saved === null) return;
  try {
    window.localStorage.setItem(STORAGE_KEY, saved);
  } catch {
    // A full or blocked store only costs this thread its resume point.
  }
}

/**
 * Turn failures reach the UI through `status` and `error`, and a rejected
 * cancel only means the turn kept running, so a rejected command needs no
 * second reporting path — just no unhandled rejection.
 */
function dispatch(command: Promise<unknown>) {
  command.catch(() => undefined);
}

function pendingRequests(
  messages: readonly EveMessage[]
): readonly PendingRequest[] {
  // An unrelated turn can append newer messages while an approval stays open,
  // so every message is scanned rather than only the last one.
  return messages.flatMap((message) =>
    message.parts.flatMap((part) => {
      if (part.type !== "dynamic-tool" || part.state !== "approval-requested") {
        return [];
      }
      const request = part.toolMetadata?.eve?.inputRequest;
      return request ? [{ request, toolName: part.toolName }] : [];
    })
  );
}

function toolSummary(part: Extract<EveMessagePart, { type: "dynamic-tool" }>) {
  switch (part.state) {
    case "input-streaming":
    case "input-available": {
      return "running";
    }
    case "approval-requested": {
      return "waiting for you";
    }
    case "approval-responded": {
      return "approved";
    }
    case "output-available": {
      return part.partial ? "running" : "done";
    }
    case "output-denied": {
      return "declined";
    }
    default: {
      return part.errorText;
    }
  }
}

function MessagePart({ part }: { readonly part: EveMessagePart }) {
  if (part.type === "text") {
    return (
      <p className="mt-2 wrap-anywhere text-sm whitespace-pre-wrap">
        {part.text}
      </p>
    );
  }
  if (part.type === "reasoning") {
    return (
      <p className="mt-2 wrap-anywhere text-[0.8125rem] whitespace-pre-wrap text-muted-foreground">
        {part.text}
      </p>
    );
  }
  if (part.type === "file") {
    return (
      <p className="mt-2 wrap-anywhere text-[0.8125rem] whitespace-pre-wrap text-muted-foreground">
        {part.filename ?? part.mediaType}
      </p>
    );
  }
  if (part.type === "authorization") {
    return (
      <p className="mt-2 wrap-anywhere text-[0.8125rem] whitespace-pre-wrap text-muted-foreground">
        {part.state === "completed"
          ? `${part.displayName} authorization ${part.outcome}.`
          : part.description}
        {part.state === "required" && part.authorization?.url ? (
          <a
            className="ml-2 text-foreground"
            href={part.authorization.url}
            rel="noreferrer"
            target="_blank"
          >
            Sign in <span aria-hidden="true">↗</span>
          </a>
        ) : null}
      </p>
    );
  }
  if (part.type === "dynamic-tool") {
    return (
      <p className="mt-2 flex items-baseline gap-2 text-xs whitespace-normal text-muted-foreground">
        <code>{part.toolName}</code>
        <span>{toolSummary(part)}</span>
      </p>
    );
  }
  return null;
}

function ChatMessage({ message }: { readonly message: EveMessage }) {
  return (
    <li
      className={`min-w-0 rounded-md border border-border p-4 ${message.role === "user" ? "bg-secondary" : ""}`}
    >
      <article aria-label={`${message.role} message`}>
        <header className="font-mono text-xs text-muted-foreground">
          {message.role === "user" ? "You" : "Factory"}
        </header>
        {message.parts.map((part, index) => (
          <MessagePart key={index} part={part} />
        ))}
      </article>
    </li>
  );
}

function ChatThread({
  agentRunsUrl,
  availableModels,
  model,
  modelsError,
  onModelChange,
  onNewChat,
  saved
}: {
  readonly agentRunsUrl: string;
  readonly availableModels: readonly AvailableModel[];
  readonly model: string;
  readonly modelsError: string | null;
  readonly onModelChange: (model: string) => void;
  readonly onNewChat: () => void;
  readonly saved: SavedChat | null;
}) {
  const savedSessionId = useRef(saved?.session.sessionId);
  const [draft, setDraft] = useState("");
  const [answers, setAnswers] = useState<Record<string, string>>({});
  const transcript = useRef<HTMLOListElement | null>(null);
  const agent = useEveAgent({
    headers: consoleHeaders(model),
    initialEvents: saved?.events,
    initialSession: saved?.session,
    onFinish: (snapshot) => writeSavedChat(snapshot),
    onSessionChange: (session) => {
      // Record a brand new session immediately, so reloading mid-turn still
      // lands back in this conversation. The event log follows on `onFinish`.
      if (session === undefined || session.sessionId === savedSessionId.current)
        return;
      savedSessionId.current = session.sessionId;
      writeSavedChat({ events: [], session });
    }
  });

  const messages = agent.data.messages;
  const isBusy = agent.status === "submitted" || agent.status === "streaming";
  const pending = pendingRequests(messages);

  useEffect(() => {
    // The transcript scrolls inside itself, so follow the newest message there
    // rather than moving the page.
    const list = transcript.current;
    if (list) list.scrollTop = list.scrollHeight;
  }, [messages]);

  function submitDraft() {
    const message = draft.trim();
    if (message.length === 0 || isBusy) return;
    setDraft("");
    dispatch(agent.send(message));
  }

  function answer(
    requestId: string,
    response: { optionId?: string; text?: string }
  ) {
    setAnswers((current) => ({ ...current, [requestId]: "" }));
    dispatch(agent.respond([{ requestId, ...response }]));
  }

  return (
    <div>
      <div className="flex flex-wrap items-center justify-between gap-4">
        <div
          className={`flex min-w-0 flex-1 basis-60 items-start gap-3 rounded-md bg-muted p-3.5 text-[0.8125rem] ${agent.status === "submitted" || agent.status === "streaming" ? "text-warning" : agent.status === "error" ? "text-destructive" : "text-success"}`}
          role="status"
        >
          <span
            className="mt-1.5 size-[7px] shrink-0 rounded-full bg-current"
            aria-hidden="true"
          />
          <div>
            <strong className="block font-semibold capitalize">
              {isBusy ? "working" : agent.status}
            </strong>
            {agent.session ? (
              <code className="mt-1 block wrap-anywhere font-mono text-xs text-muted-foreground">
                {agent.session.sessionId}
              </code>
            ) : null}
            {agent.error ? (
              <code className="mt-1 block wrap-anywhere font-mono text-xs text-muted-foreground">
                {agent.error.message}
              </code>
            ) : null}
          </div>
        </div>
        <div className="flex flex-wrap items-center gap-2.5 max-[520px]:[&>*]:w-full">
          {isBusy ? (
            <Button
              onClick={() => dispatch(agent.cancel())}
              size="sm"
              type="button"
              variant="outline"
            >
              Stop
            </Button>
          ) : null}
          <label className="grid gap-1 text-xs text-muted-foreground">
            Model
            <select
              aria-label="Model"
              className="min-h-9 max-w-64 rounded-md border border-input bg-background px-2 text-sm text-foreground focus-visible:outline-2 focus-visible:outline-ring focus-visible:outline-offset-2"
              disabled={
                isBusy ||
                agent.session !== undefined ||
                availableModels.length === 0
              }
              onChange={(event) => onModelChange(event.target.value)}
              value={model}
            >
              {availableModels.some(
                (candidate) => candidate.id === model
              ) ? null : (
                <option value={model}>{model}</option>
              )}
              {availableModels.map((candidate) => (
                <option key={candidate.id} value={candidate.id}>
                  {candidate.name} ({candidate.ownedBy})
                </option>
              ))}
            </select>
            {modelsError ? (
              <span className="text-destructive">{modelsError}</span>
            ) : null}
          </label>
          <Button
            disabled={isBusy}
            onClick={onNewChat}
            size="sm"
            type="button"
            variant="outline"
          >
            New chat
          </Button>
          <a
            className="inline-flex min-h-10 items-center gap-1.5 px-2 text-sm font-medium text-foreground no-underline hover:underline hover:underline-offset-4"
            href={agentRunsUrl}
            rel="noreferrer"
            target="_blank"
          >
            Open Agent Runs <span className="sr-only">in a new tab</span>
            <span aria-hidden="true">↗</span>
          </a>
        </div>
      </div>

      {messages.length > 0 ? (
        <ol
          className="mt-6 grid max-h-[60vh] list-none gap-4 overflow-y-auto p-0"
          ref={transcript}
        >
          {messages.map((message) => (
            <ChatMessage key={message.id} message={message} />
          ))}
        </ol>
      ) : (
        <p className="mt-6 grid min-h-[180px] place-items-center rounded-md border border-dashed border-border p-6 text-center text-sm text-muted-foreground">
          {agent.session
            ? "This session's earlier messages were too large to restore. Keep going, or start a new chat."
            : "Ask for anything in the checkout. The agent opens a pull request only when you approve it."}
        </p>
      )}

      {pending.map(({ request, toolName }) => (
        <fieldset
          className="mt-6 rounded-md border border-warning p-4"
          key={request.requestId}
        >
          <legend className="px-1.5 text-[0.8125rem] font-semibold">
            {request.kind === "tool-approval"
              ? `Approve ${toolName}`
              : request.kind === "question"
                ? "Question"
                : "Session limit"}
          </legend>
          <p className="mb-4 wrap-anywhere text-sm whitespace-pre-wrap">
            {request.prompt}
          </p>
          <div className="flex flex-wrap items-center gap-2.5 max-[520px]:[&>*]:w-full">
            {request.options?.map((option) => (
              <Button
                disabled={isBusy}
                key={option.id}
                onClick={() =>
                  answer(request.requestId, { optionId: option.id })
                }
                size="sm"
                type="button"
                variant={option.style === "danger" ? "outline" : "default"}
              >
                {option.label}
              </Button>
            ))}
          </div>
          {request.allowFreeform || request.display === "text" ? (
            <div className="mt-3 flex items-center gap-2">
              <input
                aria-label="Answer"
                className="min-h-9 min-w-0 flex-auto rounded-md border border-input bg-background px-3 text-sm text-foreground focus-visible:outline-2 focus-visible:outline-ring focus-visible:outline-offset-2"
                disabled={isBusy}
                onChange={(event) =>
                  setAnswers((current) => ({
                    ...current,
                    [request.requestId]: event.target.value
                  }))
                }
                value={answers[request.requestId] ?? ""}
              />
              <Button
                disabled={isBusy || !answers[request.requestId]?.trim()}
                onClick={() =>
                  answer(request.requestId, {
                    text: answers[request.requestId]?.trim()
                  })
                }
                size="sm"
                type="button"
                variant="outline"
              >
                Answer
              </Button>
            </div>
          ) : null}
        </fieldset>
      ))}

      <form
        className="mt-6 grid justify-items-end gap-3"
        onSubmit={(event) => {
          event.preventDefault();
          submitDraft();
        }}
      >
        <label className="sr-only" htmlFor="chat-message">
          Message
        </label>
        <textarea
          className="w-full resize-y rounded-md border border-input bg-background p-3 text-sm text-foreground focus-visible:outline-2 focus-visible:outline-ring focus-visible:outline-offset-2"
          disabled={isBusy}
          id="chat-message"
          onChange={(event) => setDraft(event.target.value)}
          onKeyDown={(event) => {
            if (event.key === "Enter" && !event.shiftKey) {
              event.preventDefault();
              submitDraft();
            }
          }}
          placeholder="Land the fix for the affected-glob warning and open a PR when I say so."
          rows={3}
          value={draft}
        />
        <Button disabled={isBusy || draft.trim().length === 0} type="submit">
          {isBusy ? "Working…" : "Send"}
        </Button>
      </form>
    </div>
  );
}

export function OperatorChat({ agentRunsUrl }: OperatorChatProps) {
  // Browser storage is only readable after hydration, so the thread mounts on
  // the first effect. Bumping `key` starts a fresh durable session.
  const [thread, setThread] = useState<{
    readonly key: number;
    readonly saved: SavedChat | null;
  } | null>(null);
  const [model, setModel] = useState(GPT_SOL_MODEL);
  const [availableModels, setAvailableModels] = useState<
    readonly AvailableModel[]
  >([]);
  const [modelsError, setModelsError] = useState<string | null>(null);

  useEffect(() => {
    let savedModel = GPT_SOL_MODEL;
    try {
      savedModel = window.localStorage.getItem(MODEL_STORAGE_KEY) ?? savedModel;
    } catch {
      // Use the default model when browser storage is unavailable.
    }
    setModel(savedModel);
    setThread({ key: 0, saved: readSavedChat() });

    const controller = new AbortController();
    fetch("/api/models", { cache: "no-store", signal: controller.signal })
      .then(async (response) => {
        if (!response.ok) throw new Error("Could not load models.");
        const payload = (await response.json()) as { models?: unknown };
        if (!Array.isArray(payload.models))
          throw new Error("Could not load models.");
        const models = payload.models.filter(
          (candidate): candidate is AvailableModel =>
            typeof candidate === "object" &&
            candidate !== null &&
            typeof (candidate as AvailableModel).id === "string" &&
            typeof (candidate as AvailableModel).name === "string" &&
            typeof (candidate as AvailableModel).ownedBy === "string"
        );
        setAvailableModels(models);
      })
      .catch((error: unknown) => {
        if (error instanceof DOMException && error.name === "AbortError")
          return;
        setModelsError("Model list unavailable");
      });
    return () => controller.abort();
  }, []);

  function changeModel(nextModel: string) {
    setModel(nextModel);
    try {
      window.localStorage.setItem(MODEL_STORAGE_KEY, nextModel);
    } catch {
      // The choice still applies to this page load.
    }
    setThread((current) => ({ key: (current?.key ?? 0) + 1, saved: null }));
  }

  function startNewChat() {
    try {
      window.localStorage.removeItem(STORAGE_KEY);
    } catch {
      // Nothing to clear.
    }
    setThread((current) => ({ key: (current?.key ?? 0) + 1, saved: null }));
  }

  if (thread === null) {
    return (
      <p className="mt-6 grid min-h-[180px] place-items-center rounded-md border border-dashed border-border p-6 text-center text-sm text-muted-foreground">
        Loading the console…
      </p>
    );
  }

  return (
    <ChatThread
      agentRunsUrl={agentRunsUrl}
      availableModels={availableModels}
      key={thread.key}
      model={model}
      modelsError={modelsError}
      onModelChange={changeModel}
      onNewChat={startNewChat}
      saved={thread.saved}
    />
  );
}
