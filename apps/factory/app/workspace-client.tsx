"use client";

import {
  Client,
  defaultMessageReducer,
  type EveMessageData,
  type MessageStreamEvent
} from "eve/client";
import type {
  EveMessage,
  EveMessageInputRequest,
  EveMessagePart
} from "eve/react";
import { useEveAgent } from "eve/react";
import dynamic from "next/dynamic";
import Link from "next/link";
import { useEffect, useState } from "react";

import {
  OPERATOR_ACTION_HEADER,
  OPERATOR_CHAT_ACTION
} from "../agent/lib/operator-console";
import { sandboxSshCommand } from "../agent/lib/sandbox-ssh";
import { CopyCommand } from "../components/copy-command";
import { Button } from "../components/ui/button";
import type { PublicWorkspace } from "./workspace-types";

const CONSOLE_HEADERS = { [OPERATOR_ACTION_HEADER]: OPERATOR_CHAT_ACTION };
const WorkspaceTerminal = dynamic(
  () =>
    import("./workspace-terminal").then((module) => module.WorkspaceTerminal),
  { ssr: false }
);

interface WorkspaceClientProps {
  readonly workspaceId: string;
}

interface LoadedWorkspace {
  readonly events: readonly MessageStreamEvent[];
  readonly workspace: PublicWorkspace;
}

interface PendingRequest {
  readonly request: EveMessageInputRequest;
  readonly toolName: string;
}

export function WorkspaceClient({ workspaceId }: WorkspaceClientProps) {
  const [loaded, setLoaded] = useState<LoadedWorkspace | null>(null);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    let cancelled = false;
    void fetch(`/api/workspaces/${encodeURIComponent(workspaceId)}`, {
      cache: "no-store"
    })
      .then(async (response) => {
        if (!response.ok)
          throw new Error(`Could not load workspace (${response.status}).`);
        return (await response.json()) as PublicWorkspace;
      })
      .then(async (workspace) => {
        if (!workspace.sessionId) return { events: [], workspace };
        const client = new Client({ headers: CONSOLE_HEADERS, host: "" });
        const snapshot = await client.sessions
          .attach(workspace.sessionId)
          .snapshot();
        return { events: snapshot.events, workspace };
      })
      .then((value) => {
        if (!cancelled) setLoaded(value);
      })
      .catch((cause) => {
        if (!cancelled)
          setError(
            cause instanceof Error ? cause.message : "Could not load workspace."
          );
      });
    return () => {
      cancelled = true;
    };
  }, [workspaceId]);

  if (error)
    return (
      <main
        className="mx-auto w-[min(900px,calc(100%_-_48px))] py-12"
        id="main-content"
      >
        <p className="text-sm text-destructive" role="alert">
          {error}
        </p>
      </main>
    );
  if (!loaded?.workspace.sessionId)
    return (
      <main className="grid min-h-[60vh] place-items-center" id="main-content">
        <p className="text-sm text-muted-foreground" role="status">
          Loading durable Eve session…
        </p>
      </main>
    );
  return (
    <WorkspaceChat initialEvents={loaded.events} workspace={loaded.workspace} />
  );
}

function WorkspaceChat({
  initialEvents,
  workspace
}: {
  readonly initialEvents: readonly MessageStreamEvent[];
  readonly workspace: PublicWorkspace;
}) {
  const [answers, setAnswers] = useState<Record<string, string>>({});
  const [draft, setDraft] = useState("");
  const [events, setEvents] = useState(initialEvents);
  const [terminalOpen, setTerminalOpen] = useState(false);
  const agent = useEveAgent({
    headers: CONSOLE_HEADERS,
    initialEvents,
    initialSession: {
      sessionId: workspace.sessionId!,
      streamIndex: initialEvents.length
    }
  });

  useEffect(() => {
    const controller = new AbortController();
    const client = new Client({ headers: CONSOLE_HEADERS, host: "" });
    const session = client.sessions.attach(workspace.sessionId!, {
      streamIndex: initialEvents.length
    });
    void (async () => {
      try {
        for await (const event of session.stream({
          signal: controller.signal
        })) {
          setEvents((current) =>
            current.some((candidate) => candidate.meta.id === event.meta.id)
              ? current
              : [...current, event]
          );
        }
      } catch (cause) {
        if (!controller.signal.aborted) throw cause;
      }
    })();
    return () => controller.abort();
  }, [initialEvents.length, workspace.sessionId]);

  const data = reduceEvents(events);
  const pending = pendingRequests(data.messages);
  const serverBusy = isServerBusy(events);
  const busy =
    serverBusy || agent.status === "submitted" || agent.status === "streaming";
  const canRespond = pending.length > 0 && agent.status !== "error";

  function submit() {
    const message = draft.trim();
    if (!message || busy) return;
    setDraft("");
    agent.send(message).catch(() => undefined);
  }

  function answer(
    requestId: string,
    response: { optionId?: string; text?: string }
  ) {
    setAnswers((current) => ({ ...current, [requestId]: "" }));
    agent.respond([{ requestId, ...response }]).catch(() => undefined);
  }

  return (
    <main
      className="mx-auto w-[min(900px,calc(100%_-_48px))] py-10"
      id="main-content"
    >
      <header className="flex items-start justify-between gap-6">
        <div>
          <Link
            className="text-xs text-muted-foreground hover:underline"
            href="/"
          >
            ← Workspaces
          </Link>
          <h1 className="mt-3 text-2xl font-semibold tracking-[-0.03em]">
            {workspace.title}
          </h1>
          <code className="mt-2 block text-xs text-muted-foreground">
            {workspace.sessionId}
          </code>
          {workspace.sandbox.id ? (
            <CopyCommand
              command={sandboxSshCommand(workspace.sandbox.id)}
              label="sandbox SSH command"
            />
          ) : null}
        </div>
        <span
          className={`mt-6 text-xs ${agent.status === "error" ? "text-destructive" : busy ? "text-warning" : "text-success"}`}
        >
          {busy ? "Streaming" : agent.status === "error" ? "Error" : "Ready"}
        </span>
      </header>

      <section className="mt-8" aria-labelledby="conversation-title">
        <h2 className="text-sm font-semibold" id="conversation-title">
          Conversation
        </h2>
        <ol className="mt-3 grid max-h-[52vh] list-none gap-4 overflow-y-auto p-0">
          {data.messages.map((message) => (
            <WorkspaceMessage key={message.id} message={message} />
          ))}
        </ol>
      </section>

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
                disabled={!canRespond}
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
                disabled={!canRespond}
                onChange={(event) =>
                  setAnswers((current) => ({
                    ...current,
                    [request.requestId]: event.target.value
                  }))
                }
                value={answers[request.requestId] ?? ""}
              />
              <Button
                disabled={!canRespond || !answers[request.requestId]?.trim()}
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

      <ActivityFeed events={events} />

      {workspace.sandbox.id ? (
        <details
          className="mt-6 rounded-md border border-border p-4"
          onToggle={(event) => setTerminalOpen(event.currentTarget.open)}
        >
          <summary className="cursor-pointer text-sm font-semibold">
            Sandbox terminal
          </summary>
          {terminalOpen ? (
            <div className="mt-4">
              <WorkspaceTerminal workspaceId={workspace.id} />
            </div>
          ) : null}
        </details>
      ) : null}

      {agent.error ? (
        <p className="mt-4 text-sm text-destructive" role="alert">
          {agent.error.message}
        </p>
      ) : null}
      <form
        className="mt-6 grid gap-3 border-t border-border pt-6"
        onSubmit={(event) => {
          event.preventDefault();
          submit();
        }}
      >
        <label className="text-sm font-medium" htmlFor="workspace-message">
          Continue the work
        </label>
        <textarea
          className="min-h-28 resize-y rounded-md border border-input bg-background p-3 text-sm"
          disabled={busy}
          id="workspace-message"
          onChange={(event) => setDraft(event.target.value)}
          placeholder="Ask Factory to inspect, change, or verify something in this workspace."
          value={draft}
        />
        <Button
          className="justify-self-start"
          disabled={busy || !draft.trim()}
          type="submit"
        >
          {busy ? "Working…" : "Send"}
        </Button>
      </form>
    </main>
  );
}

const messageReducer = defaultMessageReducer();

function pendingRequests(
  messages: readonly EveMessage[]
): readonly PendingRequest[] {
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

function reduceEvents(events: readonly MessageStreamEvent[]): EveMessageData {
  return events.reduce(
    (data, event) => messageReducer.reduce(data, event),
    messageReducer.initial()
  );
}

function isServerBusy(events: readonly MessageStreamEvent[]): boolean {
  let busy = false;
  for (const event of events) {
    if (event.type === "turn.started") busy = true;
    if (
      event.type === "turn.completed" ||
      event.type === "turn.cancelled" ||
      event.type === "turn.failed" ||
      event.type === "session.failed"
    )
      busy = false;
  }
  return busy;
}

function ActivityFeed({
  events
}: {
  readonly events: readonly MessageStreamEvent[];
}) {
  const visible = events.filter((event) =>
    [
      "message.received",
      "turn.started",
      "reasoning.completed",
      "actions.requested",
      "action.result",
      "message.completed",
      "turn.completed",
      "turn.failed",
      "session.failed",
      "session.waiting"
    ].includes(event.type)
  );
  return (
    <details className="mt-6 rounded-md border border-border p-4" open>
      <summary className="cursor-pointer text-sm font-semibold">
        Live activity ({visible.length})
      </summary>
      <ol className="mt-3 grid max-h-72 list-none gap-2 overflow-y-auto p-0 font-mono text-xs">
        {visible.map((event, index) => (
          <li
            className="grid grid-cols-[8rem_minmax(0,1fr)] gap-3 border-t border-border pt-2"
            key={event.meta.id ?? index}
          >
            <time className="text-muted-foreground" dateTime={event.meta.at}>
              {new Date(event.meta.at).toLocaleTimeString()}
            </time>
            <span className="min-w-0 wrap-anywhere">
              <strong>{event.type}</strong>
              <span className="ml-2 text-muted-foreground">
                {eventSummary(event)}
              </span>
            </span>
          </li>
        ))}
      </ol>
    </details>
  );
}

function eventSummary(event: MessageStreamEvent): string {
  switch (event.type) {
    case "message.received":
      return event.data.message;
    case "reasoning.completed":
      return event.data.reasoning;
    case "message.completed":
      return event.data.message ?? event.data.finishReason;
    case "actions.requested":
      return `${event.data.actions.length} action${event.data.actions.length === 1 ? "" : "s"} requested`;
    case "action.result":
      return event.data.error?.message ?? event.data.status;
    case "turn.failed":
    case "session.failed":
      return event.data.message;
    case "turn.started":
    case "turn.completed":
      return event.data.turnId;
    case "session.waiting":
      return "Waiting for your next message";
    default:
      return "";
  }
}

function WorkspaceMessage({ message }: { readonly message: EveMessage }) {
  return (
    <li
      className={`rounded-md border border-border p-4 ${message.role === "user" ? "bg-secondary" : ""}`}
    >
      <article>
        <header className="font-mono text-xs text-muted-foreground">
          {message.role === "user" ? "You" : "Factory"}
        </header>
        {message.parts.map((part, index) => (
          <WorkspacePart key={index} part={part} />
        ))}
      </article>
    </li>
  );
}

function WorkspacePart({ part }: { readonly part: EveMessagePart }) {
  if (part.type === "text" || part.type === "reasoning")
    return <p className="mt-2 whitespace-pre-wrap text-sm">{part.text}</p>;
  if (part.type === "dynamic-tool")
    return (
      <p className="mt-2 font-mono text-xs text-muted-foreground">
        {part.toolName}: {part.state}
      </p>
    );
  return null;
}
