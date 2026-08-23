"use client";

import dynamic from "next/dynamic";
import Link from "next/link";
import { useCallback, useEffect, useRef, useState } from "react";

import { CopyCommand } from "../components/copy-command";
import { Button } from "../components/ui/button";
import type { PublicWorkspace, WorkspaceMessage } from "./workspace-types";
import { isWorkspaceRunning } from "./workspace-types";

const WorkspaceTerminal = dynamic(
  () =>
    import("./workspace-terminal").then((module) => module.WorkspaceTerminal),
  { ssr: false }
);
const POLL_INTERVAL_MS = 3_000;
const MAX_DIFF_CHARACTERS = 60_000;
const WORKFLOW_RUNS_URL =
  "https://vercel.com/vercel-internal-apps/turborepo-factory/observability/workflows";

interface WorkspaceClientProps {
  readonly workspaceId: string;
}

function Message({ message }: { readonly message: WorkspaceMessage }) {
  const author =
    message.role === "user"
      ? "You"
      : message.role === "assistant"
        ? "Factory"
        : "System";
  return (
    <li
      className={`rounded-md border border-border p-4 ${message.role === "user" ? "bg-secondary" : ""}`}
    >
      <article aria-label={`${author} message`}>
        <header className="flex items-baseline justify-between gap-4 font-mono text-xs text-muted-foreground">
          <span>{author}</span>
          <time dateTime={message.createdAt}>
            {new Date(message.createdAt).toLocaleString()}
          </time>
        </header>
        <p className="mt-2 wrap-anywhere text-sm whitespace-pre-wrap">
          {message.text}
        </p>
      </article>
    </li>
  );
}

export function WorkspaceClient({ workspaceId }: WorkspaceClientProps) {
  const [workspace, setWorkspace] = useState<PublicWorkspace | null>(null);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);
  const [draft, setDraft] = useState("");
  const [sending, setSending] = useState(false);
  const [diff, setDiff] = useState<{
    readonly status: string;
    readonly text: string;
    readonly truncated: boolean;
  } | null>(null);
  const [diffLoading, setDiffLoading] = useState(false);
  const [audit, setAudit] = useState<{
    readonly text: string;
    readonly truncated: boolean;
  } | null>(null);
  const [auditLoading, setAuditLoading] = useState(false);
  const [terminalOpen, setTerminalOpen] = useState(false);
  const transcript = useRef<HTMLOListElement>(null);

  const refresh = useCallback(async () => {
    try {
      const response = await fetch(
        `/api/workspaces/${encodeURIComponent(workspaceId)}`,
        { cache: "no-store" }
      );
      if (!response.ok)
        throw new Error(`Could not load workspace (${response.status}).`);
      setWorkspace((await response.json()) as PublicWorkspace);
      setError(null);
    } catch (cause) {
      setError(
        cause instanceof Error ? cause.message : "Could not load workspace."
      );
    } finally {
      setLoading(false);
    }
  }, [workspaceId]);

  const refreshDiff = useCallback(async () => {
    setDiffLoading(true);
    try {
      const response = await fetch(
        `/api/workspaces/${encodeURIComponent(workspaceId)}/diff`,
        { cache: "no-store" }
      );
      if (!response.ok)
        throw new Error(`Could not load diff (${response.status}).`);
      const result = (await response.json()) as {
        readonly status: string;
        readonly diff: string;
      };
      setDiff({
        status: result.status,
        text: result.diff.slice(0, MAX_DIFF_CHARACTERS),
        truncated: result.diff.length > MAX_DIFF_CHARACTERS
      });
      setError(null);
    } catch (cause) {
      setError(cause instanceof Error ? cause.message : "Could not load diff.");
    } finally {
      setDiffLoading(false);
    }
  }, [workspaceId]);

  const refreshAudit = useCallback(async () => {
    setAuditLoading(true);
    try {
      const response = await fetch(
        `/api/workspaces/${encodeURIComponent(workspaceId)}/audit`,
        { cache: "no-store" }
      );
      if (!response.ok)
        throw new Error(`Could not load fx audit (${response.status}).`);
      const result = (await response.json()) as {
        readonly audit: string;
        readonly truncated: boolean;
      };
      setAudit({ text: result.audit, truncated: result.truncated });
      setError(null);
    } catch (cause) {
      setError(
        cause instanceof Error ? cause.message : "Could not load fx audit."
      );
    } finally {
      setAuditLoading(false);
    }
  }, [workspaceId]);

  useEffect(() => {
    void refresh();
  }, [refresh]);

  useEffect(() => {
    if (!workspace || !isWorkspaceRunning(workspace.status)) return;
    const poll = () => {
      if (document.visibilityState === "visible") void refresh();
    };
    const timer = window.setInterval(poll, POLL_INTERVAL_MS);
    document.addEventListener("visibilitychange", poll);
    return () => {
      window.clearInterval(timer);
      document.removeEventListener("visibilitychange", poll);
    };
  }, [refresh, workspace]);

  useEffect(() => {
    const list = transcript.current;
    if (list) list.scrollTop = list.scrollHeight;
  }, [workspace?.messages]);

  async function sendMessage() {
    const message = draft.trim();
    if (!message || sending || isWorkspaceRunning(workspace?.status ?? ""))
      return;
    setSending(true);
    setError(null);
    try {
      const response = await fetch(
        `/api/workspaces/${encodeURIComponent(workspaceId)}/messages`,
        {
          method: "POST",
          headers: {
            "content-type": "application/json",
            "x-operator-action": "send-workspace-message"
          },
          body: JSON.stringify({ message })
        }
      );
      if (!response.ok) {
        const body = (await response.json().catch(() => ({}))) as {
          error?: string;
        };
        throw new Error(
          body.error ?? `Could not send message (${response.status}).`
        );
      }
      setWorkspace((await response.json()) as PublicWorkspace);
      setDraft("");
    } catch (cause) {
      setError(
        cause instanceof Error ? cause.message : "Could not send message."
      );
    } finally {
      setSending(false);
    }
  }

  if (loading)
    return (
      <main
        id="main-content"
        className="mx-auto w-[min(960px,calc(100%_-_48px))] py-12"
        aria-busy="true"
      >
        Loading workspace…
      </main>
    );

  if (!workspace) {
    return (
      <main
        id="main-content"
        className="mx-auto w-[min(960px,calc(100%_-_48px))] py-12"
      >
        <p role="alert">{error ?? "Workspace not found."}</p>
        <Button
          className="mt-4"
          onClick={() => void refresh()}
          type="button"
          variant="outline"
        >
          Retry
        </Button>
      </main>
    );
  }

  const running = isWorkspaceRunning(workspace.status);
  const pullRequestUrl =
    typeof workspace.pullRequest === "string"
      ? workspace.pullRequest
      : workspace.pullRequest?.url;

  return (
    <main
      id="main-content"
      className="mx-auto w-[min(960px,calc(100%_-_48px))] py-10 max-[720px]:w-[min(960px,calc(100%_-_32px))]"
    >
      <Link
        className="text-sm text-muted-foreground hover:text-foreground"
        href="/"
      >
        ← Workspaces
      </Link>
      <header className="mt-6 flex items-start justify-between gap-6 max-[620px]:flex-col">
        <div className="min-w-0">
          <h1 className="text-pretty text-[clamp(1.5rem,3vw,2rem)] leading-tight font-semibold tracking-[-0.04em]">
            {workspace.title || "Untitled workspace"}
          </h1>
          <p className="mt-2 truncate font-mono text-xs text-muted-foreground">
            {workspace.id}
          </p>
        </div>
        <div
          className={`flex items-center gap-2 rounded-md bg-muted px-3 py-2 text-sm capitalize ${running ? "text-warning" : workspace.status === "error" ? "text-destructive" : "text-success"}`}
          role="status"
        >
          <span
            className="size-1.5 rounded-full bg-current"
            aria-hidden="true"
          />
          {workspace.status}
        </div>
      </header>

      {workspace.error || error ? (
        <div
          className="mt-6 flex items-start justify-between gap-4 rounded-md border border-destructive p-4 text-sm"
          role="alert"
        >
          <span>{error ?? workspace.error}</span>
          {error ? (
            <Button
              onClick={() => setError(null)}
              size="sm"
              type="button"
              variant="ghost"
            >
              Dismiss
            </Button>
          ) : null}
        </div>
      ) : null}

      <dl className="mt-8 grid grid-cols-3 gap-5 max-[620px]:grid-cols-1">
        <div className="border-t border-border pt-3">
          <dt className="text-xs text-muted-foreground">Agent</dt>
          <dd className="mt-1 font-mono text-sm">{workspace.agent}</dd>
        </div>
        <div className="border-t border-border pt-3">
          <dt className="text-xs text-muted-foreground">Sandbox</dt>
          <dd className="mt-1 truncate font-mono text-sm">
            {workspace.sandbox.name}
          </dd>
        </div>
        <div className="border-t border-border pt-3">
          <dt className="text-xs text-muted-foreground">Session</dt>
          <dd className="mt-1 truncate font-mono text-sm">
            {workspace.sessionId ?? "starts with first turn"}
          </dd>
        </div>
      </dl>

      <section className="mt-10" aria-labelledby="transcript-title">
        <div className="flex items-center justify-between gap-4">
          <h2 className="text-lg font-semibold" id="transcript-title">
            Transcript
          </h2>
          <Button
            disabled={loading}
            onClick={() => void refresh()}
            size="sm"
            type="button"
            variant="ghost"
          >
            Refresh
          </Button>
        </div>
        {workspace.messages.length ? (
          <ol
            className="mt-4 grid max-h-[60vh] list-none gap-3 overflow-y-auto p-0"
            ref={transcript}
          >
            {workspace.messages.map((message) => (
              <Message key={message.id} message={message} />
            ))}
          </ol>
        ) : (
          <p className="mt-4 grid min-h-36 place-items-center rounded-md border border-dashed border-border p-6 text-sm text-muted-foreground">
            No messages yet.
          </p>
        )}
        <form
          className="mt-5 grid justify-items-end gap-3"
          onSubmit={(event) => {
            event.preventDefault();
            void sendMessage();
          }}
        >
          <label className="sr-only" htmlFor="workspace-message">
            Next message
          </label>
          <textarea
            className="w-full resize-y rounded-md border border-input bg-background p-3 text-sm focus-visible:outline-2 focus-visible:outline-ring focus-visible:outline-offset-2"
            disabled={running || sending}
            id="workspace-message"
            onChange={(event) => setDraft(event.target.value)}
            placeholder={
              running ? "Factory is working…" : "Send the next instruction…"
            }
            rows={3}
            value={draft}
          />
          <Button disabled={running || sending || !draft.trim()} type="submit">
            {sending ? "Sending…" : running ? "Working…" : "Send"}
          </Button>
        </form>
      </section>

      <section
        className="mt-12 border-t border-border pt-8"
        aria-labelledby="changes-title"
      >
        <div className="flex items-center justify-between gap-4">
          <div>
            <h2 className="text-lg font-semibold" id="changes-title">
              Changes
            </h2>
            {diff ? (
              <p className="mt-1 text-xs text-muted-foreground">
                {diff.status}
              </p>
            ) : null}
          </div>
          <Button
            disabled={diffLoading}
            onClick={() => void refreshDiff()}
            size="sm"
            type="button"
            variant="outline"
          >
            {diffLoading ? "Refreshing…" : diff ? "Refresh diff" : "Load diff"}
          </Button>
        </div>
        {diff ? (
          diff.text ? (
            <>
              <pre className="mt-4 max-h-[50vh] overflow-auto rounded-md bg-secondary p-4 font-mono text-xs whitespace-pre-wrap">
                {diff.text}
              </pre>
              {diff.truncated ? (
                <p className="mt-2 text-xs text-warning">
                  Diff capped at {MAX_DIFF_CHARACTERS.toLocaleString()}{" "}
                  characters.
                </p>
              ) : null}
            </>
          ) : (
            <p className="mt-4 text-sm text-muted-foreground">No changes.</p>
          )
        ) : (
          <p className="mt-4 text-sm text-muted-foreground">
            Load the current capped diff on demand.
          </p>
        )}
      </section>

      <section
        className="mt-12 border-t border-border pt-8"
        aria-labelledby="audit-title"
      >
        <div className="flex items-center justify-between gap-4">
          <div>
            <h2 className="text-lg font-semibold" id="audit-title">
              fx audit
            </h2>
            <p className="mt-1 text-xs text-muted-foreground">
              Complete saved fx session state and tool history.
            </p>
          </div>
          <Button
            disabled={auditLoading || !workspace.sessionId}
            onClick={() => void refreshAudit()}
            size="sm"
            type="button"
            variant="outline"
          >
            {auditLoading ? "Loading…" : audit ? "Refresh audit" : "Load audit"}
          </Button>
        </div>
        {audit ? (
          <>
            <pre className="mt-4 max-h-[50vh] overflow-auto rounded-md bg-secondary p-4 font-mono text-xs whitespace-pre-wrap">
              {audit.text}
            </pre>
            {audit.truncated ? (
              <p className="mt-2 text-xs text-warning">
                Audit display capped at 2,000,000 characters.
              </p>
            ) : null}
          </>
        ) : (
          <p className="mt-4 text-sm text-muted-foreground">
            {workspace.sessionId
              ? "Load the durable fx session audit on demand."
              : "The audit is available after the first fx turn starts."}
          </p>
        )}
      </section>

      <section
        className="mt-12 border-t border-border pt-8"
        aria-labelledby="access-title"
      >
        <h2 className="text-lg font-semibold" id="access-title">
          Sandbox access
        </h2>
        <CopyCommand
          command={`sandbox ssh ${workspace.sandbox.name}`}
          label="SSH command for this workspace"
        />
        {workspace.chatCommand ? (
          <CopyCommand
            command={workspace.chatCommand}
            label="Resume this agent chat after connecting"
          />
        ) : null}
        <Button
          className="mt-3"
          disabled={
            !workspace.sandbox.name || workspace.sandbox.status === "pending"
          }
          onClick={() => setTerminalOpen(true)}
          type="button"
          variant="outline"
        >
          Open browser terminal
        </Button>
        {pullRequestUrl ? (
          <p className="mt-5">
            <a
              className="text-sm font-medium underline underline-offset-4"
              href={pullRequestUrl}
              rel="noreferrer"
              target="_blank"
            >
              Open pull request <span className="sr-only">in a new tab</span> ↗
            </a>
          </p>
        ) : null}
        {workspace.workflowRunId ? (
          <p className="mt-5">
            <a
              className="text-sm font-medium underline underline-offset-4"
              href={WORKFLOW_RUNS_URL}
              rel="noreferrer"
              target="_blank"
            >
              Open full execution audit{" "}
              <span className="sr-only">in a new tab</span>
              <span aria-hidden="true">↗</span>
            </a>
          </p>
        ) : null}
      </section>

      {terminalOpen ? (
        <WorkspaceTerminal
          onExit={() => setTerminalOpen(false)}
          workspaceId={workspace.id}
          workspaceTitle={workspace.title || workspace.id}
        />
      ) : null}
    </main>
  );
}
