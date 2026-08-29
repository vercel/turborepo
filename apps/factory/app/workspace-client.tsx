"use client";

import { cjk } from "@streamdown/cjk";
import { code } from "@streamdown/code";
import { math } from "@streamdown/math";
import { mermaid } from "@streamdown/mermaid";
import {
  Client,
  defaultMessageReducer,
  type MessageStreamEvent
} from "eve/client";
import type {
  EveDynamicToolPart,
  EveMessage,
  EveMessageInputRequest,
  EveMessagePart
} from "eve/react";
import { useEveAgent } from "eve/react";
import {
  ArrowDownIcon,
  ArrowLeftIcon,
  ArrowUpIcon,
  CheckIcon,
  ChevronRightIcon,
  CircleAlertIcon,
  Loader2Icon,
  SquareIcon,
  XIcon
} from "lucide-react";
import dynamic from "next/dynamic";
import Link from "next/link";
import {
  useCallback,
  useEffect,
  useMemo,
  useRef,
  useState,
  type FormEvent,
  type KeyboardEvent
} from "react";
import { Streamdown } from "streamdown";
import { StickToBottom, useStickToBottomContext } from "use-stick-to-bottom";

import {
  OPERATOR_ACTION_HEADER,
  OPERATOR_SESSION_ACTION
} from "../agent/lib/operator-console";
import { sandboxSshCommand } from "../agent/lib/sandbox-ssh";
import { CopyCommand } from "../components/copy-command";
import { Button } from "../components/ui/button";
import type { PublicWorkspace } from "./workspace-types";

const CONSOLE_HEADERS = { [OPERATOR_ACTION_HEADER]: OPERATOR_SESSION_ACTION };
const streamdownPlugins = { cjk, code, math, mermaid };
const messageReducer = defaultMessageReducer();
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

type InputResponse = {
  readonly optionId?: string;
  readonly requestId: string;
  readonly text?: string;
};

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
      <main className="grid min-h-[60vh] place-items-center" id="main-content">
        <p className="text-sm text-destructive" role="alert">
          {error}
        </p>
      </main>
    );
  if (!loaded?.workspace.sessionId)
    return (
      <main className="grid min-h-[60vh] place-items-center" id="main-content">
        <p className="shimmer-text text-sm" role="status">
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
  const [draft, setDraft] = useState("");
  const [externalEvents, setExternalEvents] = useState(initialEvents);
  const [optimisticMessage, setOptimisticMessage] = useState<string | null>(
    null
  );
  const [stopping, setStopping] = useState(false);
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
    const session = new Client({
      headers: CONSOLE_HEADERS,
      host: ""
    }).sessions.attach(workspace.sessionId!, {
      streamIndex: initialEvents.length
    });
    void (async () => {
      try {
        for await (const event of session.stream({
          signal: controller.signal
        })) {
          setExternalEvents((current) => appendUniqueEvent(current, event));
        }
      } catch (cause) {
        if (!controller.signal.aborted) console.error(cause);
      }
    })();
    return () => controller.abort();
  }, [initialEvents.length, workspace.sessionId]);

  const events = useMemo(
    () => mergeEvents(externalEvents, agent.events),
    [agent.events, externalEvents]
  );
  const data = useMemo(
    () =>
      events.reduce(
        (current, event) => messageReducer.reduce(current, event),
        messageReducer.initial()
      ),
    [events]
  );
  const messages = useMemo(
    () =>
      appendOptimisticMessage(data.messages, optimisticMessage, workspace.id),
    [data.messages, optimisticMessage, workspace.id]
  );
  const serverBusy = isServerBusy(events);
  const busy =
    serverBusy || agent.status === "submitted" || agent.status === "streaming";
  const hasAssistantProgress = hasRenderableAssistantProgress(messages.at(-1));

  useEffect(() => {
    if (
      optimisticMessage &&
      hasLatestUserMessage(data.messages, optimisticMessage)
    ) {
      setOptimisticMessage(null);
    }
  }, [data.messages, optimisticMessage]);

  const submit = useCallback(async () => {
    const message = draft.trim();
    if (!message || busy) return;
    setDraft("");
    setOptimisticMessage(message);
    try {
      await agent.send(message);
    } catch {
      setOptimisticMessage(null);
      setDraft(message);
    }
  }, [agent, busy, draft]);

  const answer = useCallback(
    async (response: InputResponse) => {
      try {
        await agent.respond([response]);
      } catch {
        // The agent exposes the actionable failure below the conversation.
      }
    },
    [agent]
  );

  const stop = useCallback(async () => {
    if (stopping) return;
    setStopping(true);
    try {
      await agent.cancel();
    } finally {
      setStopping(false);
    }
  }, [agent, stopping]);

  return (
    <main
      className="mx-auto flex h-screen min-h-[640px] w-full max-w-5xl flex-col overflow-hidden max-[720px]:h-[calc(100dvh-113px)] max-[720px]:min-h-[520px]"
      id="main-content"
    >
      <header className="shrink-0 border-b border-border/70 px-6 py-4 max-[520px]:px-4">
        <div className="flex items-start justify-between gap-5">
          <div className="min-w-0">
            <Link
              className="inline-flex items-center gap-1 text-xs text-muted-foreground transition-colors hover:text-foreground"
              href="/"
            >
              <ArrowLeftIcon className="size-3" />
              Workspaces
            </Link>
            <h1 className="mt-1 truncate text-lg font-semibold tracking-[-0.025em]">
              {workspace.title}
            </h1>
            <p className="mt-0.5 truncate font-mono text-[11px] text-muted-foreground">
              {workspace.sessionId}
            </p>
          </div>
          <div className="flex shrink-0 items-center gap-3 pt-1">
            <span
              className={`flex items-center gap-1.5 text-xs ${agent.status === "error" ? "text-destructive" : busy ? "text-muted-foreground" : "text-success"}`}
              role="status"
            >
              {busy ? <Loader2Icon className="size-3 animate-spin" /> : null}
              {busy ? "Working" : agent.status === "error" ? "Error" : "Ready"}
            </span>
            <details className="relative">
              <summary className="cursor-pointer list-none rounded-md border border-border px-2 py-1 text-xs text-muted-foreground hover:text-foreground">
                Details
              </summary>
              <div className="absolute top-8 right-0 z-20 w-[min(34rem,calc(100vw-2rem))] rounded-md border border-border bg-popover p-3 shadow-lg">
                {workspace.sandbox.id ? (
                  <CopyCommand
                    command={sandboxSshCommand(workspace.sandbox.id)}
                    label="sandbox SSH command"
                  />
                ) : null}
                <ActivityFeed events={events} />
                {workspace.sandbox.id ? (
                  <details
                    className="mt-3 border-t border-border pt-3"
                    onToggle={(event) =>
                      setTerminalOpen(event.currentTarget.open)
                    }
                  >
                    <summary className="cursor-pointer text-xs font-medium">
                      Sandbox terminal
                    </summary>
                    {terminalOpen ? (
                      <div className="mt-3 h-72">
                        <WorkspaceTerminal workspaceId={workspace.id} />
                      </div>
                    ) : null}
                  </details>
                ) : null}
              </div>
            </details>
          </div>
        </div>
      </header>

      <ChatConversation>
        <StickToBottom.Content className="mx-auto flex w-full max-w-3xl flex-col gap-5 px-6 py-7 max-[520px]:px-4">
          {messages.map((message, index) => (
            <WorkspaceMessage
              canRespond={agent.status !== "error"}
              isStreaming={busy && index === messages.length - 1}
              key={message.id}
              message={message}
              onRespond={answer}
            />
          ))}
          {busy && !hasAssistantProgress ? <ThinkingMessage /> : null}
        </StickToBottom.Content>
        <ScrollToBottomButton />
      </ChatConversation>

      <div className="shrink-0 bg-background px-6 pt-2 pb-5 max-[520px]:px-4">
        {agent.error ? (
          <div
            className="mx-auto mb-2 flex max-w-3xl items-start gap-2 rounded-md bg-destructive/10 px-3 py-2 text-xs text-destructive"
            role="alert"
          >
            <CircleAlertIcon className="mt-0.5 size-3.5 shrink-0" />
            {agent.error.message}
          </div>
        ) : null}
        <ChatComposer
          busy={busy}
          disabled={agent.status === "error"}
          onChange={setDraft}
          onStop={stop}
          onSubmit={submit}
          stopping={stopping}
          value={draft}
        />
      </div>
    </main>
  );
}

function ChatConversation({
  children
}: {
  readonly children: React.ReactNode;
}) {
  return (
    <StickToBottom
      className="relative min-h-0 flex-1 overflow-y-hidden"
      initial="instant"
      resize="instant"
      role="log"
    >
      {children}
    </StickToBottom>
  );
}

function ScrollToBottomButton() {
  const { isAtBottom, scrollToBottom } = useStickToBottomContext();
  if (isAtBottom) return null;
  return (
    <button
      aria-label="Scroll to latest message"
      className="absolute bottom-3 left-1/2 grid size-8 -translate-x-1/2 place-items-center rounded-full border border-border bg-background text-muted-foreground shadow-sm transition-colors hover:text-foreground"
      onClick={() => scrollToBottom()}
      type="button"
    >
      <ArrowDownIcon className="size-4" />
    </button>
  );
}

function ChatComposer({
  busy,
  disabled,
  onChange,
  onStop,
  onSubmit,
  stopping,
  value
}: {
  readonly busy: boolean;
  readonly disabled: boolean;
  readonly onChange: (value: string) => void;
  readonly onStop: () => void;
  readonly onSubmit: () => void;
  readonly stopping: boolean;
  readonly value: string;
}) {
  const textareaRef = useRef<HTMLTextAreaElement>(null);

  useEffect(() => {
    textareaRef.current?.focus({ preventScroll: true });
  }, []);

  function submit(event?: FormEvent) {
    event?.preventDefault();
    onSubmit();
    if (textareaRef.current) textareaRef.current.style.height = "auto";
  }

  function keyDown(event: KeyboardEvent<HTMLTextAreaElement>) {
    if (
      event.key === "Enter" &&
      !event.shiftKey &&
      !event.nativeEvent.isComposing
    ) {
      event.preventDefault();
      submit();
    }
  }

  return (
    <form
      className="mx-auto max-w-3xl rounded-[14px] border border-border bg-card shadow-sm transition-[border-color,box-shadow] focus-within:border-foreground/20 focus-within:ring-2 focus-within:ring-foreground/5"
      onSubmit={submit}
    >
      <label className="sr-only" htmlFor="workspace-message">
        Continue the work
      </label>
      <textarea
        className="max-h-40 min-h-14 w-full resize-none bg-transparent px-4 pt-3 text-[15px] leading-6 outline-none placeholder:text-muted-foreground/60 disabled:opacity-60"
        disabled={disabled}
        id="workspace-message"
        onChange={(event) => {
          onChange(event.target.value);
          event.target.style.height = "auto";
          event.target.style.height = `${Math.min(event.target.scrollHeight, 160)}px`;
        }}
        onKeyDown={keyDown}
        placeholder="Ask Factory to inspect, change, or verify something…"
        ref={textareaRef}
        rows={1}
        value={value}
      />
      <div className="flex min-h-10 items-center justify-between px-3 pb-2">
        <span className="text-[11px] text-muted-foreground/70">
          Enter to send · Shift+Enter for a new line
        </span>
        {busy ? (
          <button
            aria-label={stopping ? "Stopping response" : "Stop response"}
            className="grid size-7 place-items-center rounded-md bg-foreground/15 text-foreground/60 transition-colors hover:bg-foreground/25 disabled:opacity-50"
            disabled={stopping}
            onClick={onStop}
            type="button"
          >
            {stopping ? (
              <Loader2Icon className="size-3.5 animate-spin" />
            ) : (
              <SquareIcon className="size-3 fill-current" />
            )}
          </button>
        ) : (
          <button
            aria-label="Send message"
            className="grid size-7 place-items-center rounded-md bg-foreground text-background transition-colors hover:bg-foreground/90 disabled:opacity-30"
            disabled={disabled || !value.trim()}
            type="submit"
          >
            <ArrowUpIcon className="size-4" />
          </button>
        )}
      </div>
    </form>
  );
}

function WorkspaceMessage({
  canRespond,
  isStreaming,
  message,
  onRespond
}: {
  readonly canRespond: boolean;
  readonly isStreaming: boolean;
  readonly message: EveMessage;
  readonly onRespond: (response: InputResponse) => void;
}) {
  const isUser = message.role === "user";
  return (
    <article
      className={`flex min-w-0 ${isUser ? "justify-end" : "justify-start"} ${message.metadata?.optimistic ? "opacity-80" : ""}`}
    >
      <div
        className={
          isUser
            ? "max-w-[85%] rounded-[18px] border border-border/50 bg-muted/70 px-3 py-1.5 text-[15px] leading-6 shadow-sm"
            : "w-full min-w-0"
        }
      >
        {message.parts.map((part, index) => (
          <WorkspacePart
            canRespond={canRespond}
            isStreaming={isStreaming}
            key={partKey(part, index)}
            onRespond={onRespond}
            part={part}
          />
        ))}
      </div>
    </article>
  );
}

function WorkspacePart({
  canRespond,
  isStreaming,
  onRespond,
  part
}: {
  readonly canRespond: boolean;
  readonly isStreaming: boolean;
  readonly onRespond: (response: InputResponse) => void;
  readonly part: EveMessagePart;
}) {
  if (part.type === "text")
    return (
      <Streamdown
        caret={isStreaming ? "block" : undefined}
        className="min-w-0 text-[15px] leading-6 [&>*:first-child]:mt-0 [&>*:last-child]:mb-0"
        isAnimating={isStreaming}
        plugins={streamdownPlugins}
      >
        {part.text}
      </Streamdown>
    );
  if (part.type === "reasoning")
    return (
      <details
        className="my-2 text-sm text-muted-foreground"
        open={part.state === "streaming"}
      >
        <summary
          className={
            part.state === "streaming" ? "shimmer-text" : "cursor-pointer"
          }
        >
          {part.state === "streaming" ? "Thinking…" : "Reasoning"}
        </summary>
        <Streamdown
          className="mt-2 border-l border-border pl-3"
          plugins={streamdownPlugins}
        >
          {part.text}
        </Streamdown>
      </details>
    );
  if (part.type === "dynamic-tool")
    return (
      <ToolCall canRespond={canRespond} onRespond={onRespond} part={part} />
    );
  return null;
}

function ToolCall({
  canRespond,
  onRespond,
  part
}: {
  readonly canRespond: boolean;
  readonly onRespond: (response: InputResponse) => void;
  readonly part: EveDynamicToolPart;
}) {
  const request = part.toolMetadata?.eve?.inputRequest;
  const running = ![
    "output-available",
    "output-denied",
    "output-error"
  ].includes(part.state);
  const failed =
    part.state === "output-error" || part.state === "output-denied";
  const details =
    part.input !== undefined || "output" in part || Boolean(request);
  return (
    <details className="my-2 px-3" open={Boolean(request)}>
      <summary
        className={`flex list-none items-center gap-2 text-sm text-muted-foreground ${details ? "cursor-pointer hover:text-foreground" : ""}`}
      >
        {running ? (
          <Loader2Icon className="size-3.5 shrink-0 animate-spin" />
        ) : failed ? (
          <XIcon className="size-3.5 shrink-0 text-destructive" />
        ) : (
          <CheckIcon className="size-3.5 shrink-0 text-success" />
        )}
        <span className="truncate">{describeTool(part)}</span>
        {details ? <ChevronRightIcon className="size-3 shrink-0" /> : null}
      </summary>
      <div className="mt-2 ml-1 space-y-2 border-l border-border pl-4">
        {request ? (
          <InputRequest
            canRespond={canRespond}
            onRespond={onRespond}
            request={request}
          />
        ) : null}
        <ToolPayload label="Input" value={part.input} />
        {part.state === "output-available" ? (
          <ToolPayload label="Result" value={part.output} />
        ) : null}
        {part.state === "output-error" ? (
          <ToolPayload label="Error" value={part.errorText} />
        ) : null}
      </div>
    </details>
  );
}

function InputRequest({
  canRespond,
  onRespond,
  request
}: {
  readonly canRespond: boolean;
  readonly onRespond: (response: InputResponse) => void;
  readonly request: EveMessageInputRequest;
}) {
  const [answer, setAnswer] = useState("");
  return (
    <div className="space-y-3 rounded-md border border-warning/40 bg-warning/5 p-3">
      <p className="text-sm text-foreground">{request.prompt}</p>
      {request.options?.length ? (
        <div className="flex flex-wrap gap-2">
          {request.options.map((option) => (
            <Button
              disabled={!canRespond}
              key={option.id}
              onClick={() =>
                onRespond({ optionId: option.id, requestId: request.requestId })
              }
              size="sm"
              type="button"
              variant={option.style === "danger" ? "outline" : "default"}
            >
              {option.label}
            </Button>
          ))}
        </div>
      ) : null}
      {request.allowFreeform || request.display === "text" ? (
        <div className="flex gap-2">
          <input
            className="min-w-0 flex-1 rounded-md border border-input bg-background px-3 text-sm outline-none focus:ring-2 focus:ring-ring/30"
            disabled={!canRespond}
            onChange={(event) => setAnswer(event.target.value)}
            onKeyDown={(event) => {
              if (event.key === "Enter" && answer.trim()) {
                onRespond({
                  requestId: request.requestId,
                  text: answer.trim()
                });
                setAnswer("");
              }
            }}
            placeholder="Type a response"
            value={answer}
          />
          <Button
            disabled={!canRespond || !answer.trim()}
            onClick={() => {
              onRespond({ requestId: request.requestId, text: answer.trim() });
              setAnswer("");
            }}
            size="sm"
            type="button"
          >
            Reply
          </Button>
        </div>
      ) : null}
    </div>
  );
}

function ToolPayload({
  label,
  value
}: {
  readonly label: string;
  readonly value: unknown;
}) {
  if (value === undefined) return null;
  return (
    <div>
      <p className="mb-1 text-[11px] text-muted-foreground">{label}</p>
      <pre className="max-h-56 overflow-auto rounded-md bg-muted/40 p-2 font-mono text-[11px] leading-5 text-muted-foreground">
        {formatPayload(value)}
      </pre>
    </div>
  );
}

function ThinkingMessage() {
  return (
    <div
      className="px-3 text-[15px] leading-6 text-muted-foreground"
      role="status"
    >
      <span className="shimmer-text">Thinking…</span>
    </div>
  );
}

function ActivityFeed({
  events
}: {
  readonly events: readonly MessageStreamEvent[];
}) {
  const visible = events.filter((event) =>
    [
      "turn.started",
      "reasoning.completed",
      "actions.requested",
      "action.result",
      "message.completed",
      "turn.completed",
      "turn.failed",
      "session.failed"
    ].includes(event.type)
  );
  return (
    <details className="mt-3 border-t border-border pt-3">
      <summary className="cursor-pointer text-xs font-medium">
        Live activity ({visible.length})
      </summary>
      <ol className="mt-2 grid max-h-52 list-none gap-1.5 overflow-y-auto p-0 font-mono text-[10px]">
        {visible.map((event, index) => (
          <li className="flex gap-2" key={event.meta.id ?? index}>
            <time
              className="shrink-0 text-muted-foreground"
              dateTime={event.meta.at}
            >
              {new Date(event.meta.at).toLocaleTimeString()}
            </time>
            <span className="min-w-0 truncate">{event.type}</span>
          </li>
        ))}
      </ol>
    </details>
  );
}

function appendUniqueEvent(
  events: readonly MessageStreamEvent[],
  event: MessageStreamEvent
): MessageStreamEvent[] {
  return events.some((candidate) => candidate.meta.id === event.meta.id)
    ? (events as MessageStreamEvent[])
    : [...events, event];
}

function mergeEvents(
  initial: readonly MessageStreamEvent[],
  streamed: readonly MessageStreamEvent[]
): MessageStreamEvent[] {
  return streamed.reduce(appendUniqueEvent, [...initial]);
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

function appendOptimisticMessage(
  messages: readonly EveMessage[],
  text: string | null,
  workspaceId: string
): readonly EveMessage[] {
  if (!text || hasLatestUserMessage(messages, text)) return messages;
  return [
    ...messages,
    {
      id: `${workspaceId}:optimistic-user-message`,
      metadata: { optimistic: true, status: "submitted" },
      parts: [{ state: "done", text, type: "text" }],
      role: "user"
    }
  ];
}

function hasLatestUserMessage(messages: readonly EveMessage[], text: string) {
  return (
    [...messages]
      .reverse()
      .find((message) => message.role === "user")
      ?.parts.filter((part) => part.type === "text")
      .map((part) => part.text)
      .join("\n")
      .trim() === text.trim()
  );
}

function hasRenderableAssistantProgress(message: EveMessage | undefined) {
  return (
    message?.role === "assistant" &&
    message.parts.some((part) => {
      if (part.type === "text" || part.type === "reasoning")
        return part.text.length > 0;
      return part.type !== "step-start";
    })
  );
}

function describeTool(part: EveDynamicToolPart) {
  const name = part.toolMetadata?.eve?.name ?? part.toolName;
  const input =
    part.input && typeof part.input === "object"
      ? (part.input as Record<string, unknown>)
      : null;
  const detail = ["command", "filePath", "path", "query", "url"]
    .map((key) => input?.[key])
    .find((value) => typeof value === "string");
  return detail ? String(detail).replace(/\s+/g, " ").slice(0, 100) : name;
}

function formatPayload(value: unknown) {
  const text =
    typeof value === "string"
      ? value
      : (() => {
          try {
            return JSON.stringify(value, null, 2);
          } catch {
            return String(value);
          }
        })();
  return text.length > 4000 ? `${text.slice(0, 4000)}\n…` : text;
}

function partKey(part: EveMessagePart, index: number) {
  return part.type === "dynamic-tool"
    ? part.toolCallId
    : `${part.type}:${index}`;
}
