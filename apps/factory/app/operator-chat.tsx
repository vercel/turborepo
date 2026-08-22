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
  OPERATOR_CHAT_ACTION
} from "../agent/lib/operator-console";
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
// Marks every eve request as coming from this page; the eve channel's
// operator-console auth entry requires it. See `agent/lib/operator-console.ts`.
const CONSOLE_HEADERS = { [OPERATOR_ACTION_HEADER]: OPERATOR_CHAT_ACTION };

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
    return <p className="chatText">{part.text}</p>;
  }
  if (part.type === "reasoning") {
    return <p className="chatReasoning">{part.text}</p>;
  }
  if (part.type === "file") {
    return <p className="chatArtifact">{part.filename ?? part.mediaType}</p>;
  }
  if (part.type === "authorization") {
    return (
      <p className="chatArtifact">
        {part.state === "completed"
          ? `${part.displayName} authorization ${part.outcome}.`
          : part.description}
        {part.state === "required" && part.authorization?.url ? (
          <a href={part.authorization.url} rel="noreferrer" target="_blank">
            Sign in <span aria-hidden="true">↗</span>
          </a>
        ) : null}
      </p>
    );
  }
  if (part.type === "dynamic-tool") {
    return (
      <p className="chatTool">
        <code>{part.toolName}</code>
        <span>{toolSummary(part)}</span>
      </p>
    );
  }
  return null;
}

function ChatMessage({ message }: { readonly message: EveMessage }) {
  return (
    <li className={`chatMessage chatMessage-${message.role}`}>
      <article aria-label={`${message.role} message`}>
        <header>{message.role === "user" ? "You" : "Factory"}</header>
        {message.parts.map((part, index) => (
          <MessagePart key={index} part={part} />
        ))}
      </article>
    </li>
  );
}

function ChatThread({
  agentRunsUrl,
  onNewChat,
  saved
}: {
  readonly agentRunsUrl: string;
  readonly onNewChat: () => void;
  readonly saved: SavedChat | null;
}) {
  const savedSessionId = useRef(saved?.session.sessionId);
  const [draft, setDraft] = useState("");
  const [answers, setAnswers] = useState<Record<string, string>>({});
  const transcript = useRef<HTMLOListElement | null>(null);
  const agent = useEveAgent({
    headers: CONSOLE_HEADERS,
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
    <div className="chat">
      <div className="chatToolbar">
        <div className={`status status-${agent.status}`} role="status">
          <span className="statusDot" aria-hidden="true" />
          <div>
            <strong>{isBusy ? "working" : agent.status}</strong>
            {agent.session ? <code>{agent.session.sessionId}</code> : null}
            {agent.error ? <code>{agent.error.message}</code> : null}
          </div>
        </div>
        <div className="actions">
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
          <Button
            disabled={isBusy}
            onClick={onNewChat}
            size="sm"
            type="button"
            variant="outline"
          >
            New chat
          </Button>
          <a href={agentRunsUrl} rel="noreferrer" target="_blank">
            Open Agent Runs <span className="visuallyHidden">in a new tab</span>
            <span aria-hidden="true">↗</span>
          </a>
        </div>
      </div>

      {messages.length > 0 ? (
        <ol className="chatTranscript" ref={transcript}>
          {messages.map((message) => (
            <ChatMessage key={message.id} message={message} />
          ))}
        </ol>
      ) : (
        <p className="emptyRunway">
          {agent.session
            ? "This session's earlier messages were too large to restore. Keep going, or start a new chat."
            : "Ask for anything in the checkout. The agent opens a pull request only when you approve it."}
        </p>
      )}

      {pending.map(({ request, toolName }) => (
        <fieldset className="chatRequest" key={request.requestId}>
          <legend>
            {request.kind === "tool-approval"
              ? `Approve ${toolName}`
              : request.kind === "question"
                ? "Question"
                : "Session limit"}
          </legend>
          <p>{request.prompt}</p>
          <div className="actions">
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
            <div className="chatAnswer">
              <input
                aria-label="Answer"
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
        className="chatComposer"
        onSubmit={(event) => {
          event.preventDefault();
          submitDraft();
        }}
      >
        <label className="visuallyHidden" htmlFor="chat-message">
          Message
        </label>
        <textarea
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

  useEffect(() => {
    setThread({ key: 0, saved: readSavedChat() });
  }, []);

  function startNewChat() {
    try {
      window.localStorage.removeItem(STORAGE_KEY);
    } catch {
      // Nothing to clear.
    }
    setThread((current) => ({ key: (current?.key ?? 0) + 1, saved: null }));
  }

  if (thread === null) {
    return <p className="emptyRunway">Loading the console…</p>;
  }

  return (
    <ChatThread
      agentRunsUrl={agentRunsUrl}
      key={thread.key}
      onNewChat={startNewChat}
      saved={thread.saved}
    />
  );
}
