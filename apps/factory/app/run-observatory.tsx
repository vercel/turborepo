"use client";

import {
  startTransition,
  useEffect,
  useEffectEvent,
  useRef,
  useState
} from "react";

import { sandboxSshCommand } from "../agent/lib/sandbox-ssh";
import type {
  AgentRunRecord,
  ControlPlaneSnapshot,
  SandboxResource
} from "../agent/lib/run-types";
import { CopyCommand } from "../components/copy-command";
import { Button } from "../components/ui/button";

const AGENT_RUNS_URL =
  "https://vercel.com/vercel-internal-apps/turborepo-eve-agent/observability/agent-runs";
const WORKFLOW_RUNS_URL =
  "https://vercel.com/vercel-internal-apps/turborepo-eve-agent/observability/workflows";
const POLL_INTERVAL_MS = 15_000;

const BOARD_COLUMNS = [
  {
    empty: "No tickets are waiting.",
    key: "queued",
    statuses: ["waiting"],
    title: "Queued"
  },
  {
    empty: "No tickets are running.",
    key: "running",
    statuses: ["running"],
    title: "In progress"
  },
  {
    empty: "No tickets have finished.",
    key: "finished",
    statuses: ["completed", "failed"],
    title: "Finished"
  }
] as const;

function formatTimestamp(value: string | number): string {
  return `${new Intl.DateTimeFormat("en-US", {
    dateStyle: "medium",
    timeStyle: "short",
    timeZone: "UTC"
  }).format(new Date(value))} UTC`;
}

function duration(run: AgentRunRecord): string {
  if (run.status === "running" || run.status === "waiting") return "live";
  const milliseconds =
    new Date(run.finishedAt ?? run.updatedAt).getTime() -
    new Date(run.startedAt).getTime();
  if (milliseconds < 60_000)
    return `${Math.max(0, Math.round(milliseconds / 1000))}s`;
  return `${Math.round(milliseconds / 60_000)}m`;
}

function RunTicket({ run }: { readonly run: AgentRunRecord }) {
  const detailsUrl = run.source === "eve" ? AGENT_RUNS_URL : WORKFLOW_RUNS_URL;
  return (
    <li className={`runTicket runTicket-${run.status}`}>
      <article aria-label={`${run.title}, ${run.status}`}>
        <header className="runTicketHeader">
          <span className="triggerLabel">{run.trigger}</span>
          <span className={`runState runState-${run.status}`}>
            <span className="statusDot" aria-hidden="true" />
            <span>{run.status}</span>
          </span>
        </header>
        <div className="runIdentity">
          <h4>{run.title}</h4>
          <code>{run.id}</code>
        </div>
        <dl className="runMetadata">
          <div>
            <dt>Runtime</dt>
            <dd>{run.harness ?? run.agent}</dd>
          </div>
          <div>
            <dt>Duration</dt>
            <dd>{duration(run)}</dd>
          </div>
          <div>
            <dt>Started</dt>
            <dd>
              <time
                dateTime={run.startedAt}
                title={formatTimestamp(run.startedAt)}
              >
                {formatTimestamp(run.startedAt)}
              </time>
            </dd>
          </div>
        </dl>
        <div className="sandboxTrack">
          <span>{run.source}</span>
          <span aria-hidden="true">→</span>
          <span>{run.sandbox?.provider ?? "no sandbox"}</span>
          <span aria-hidden="true">→</span>
          <span>{run.sandbox?.status ?? run.status}</span>
        </div>
        {run.sandbox ? (
          <CopyCommand
            command={sandboxSshCommand(run.sandbox.id)}
            label="SSH command for this sandbox"
          />
        ) : null}
        <a
          className="runDetails"
          href={detailsUrl}
          rel="noreferrer"
          target="_blank"
        >
          Inspect ticket <span className="visuallyHidden">in a new tab</span>
          <span aria-hidden="true">↗</span>
        </a>
      </article>
    </li>
  );
}

function SandboxCard({ sandbox }: { readonly sandbox: SandboxResource }) {
  return (
    <li className="sandboxCard">
      <div>
        <span
          className={`resourceDot resourceDot-${sandbox.status}`}
          aria-hidden="true"
        />
        <strong>{sandbox.name}</strong>
      </div>
      <dl>
        <div>
          <dt>Status</dt>
          <dd>{sandbox.status}</dd>
        </div>
        <div>
          <dt>Runtime</dt>
          <dd>{sandbox.runtime ?? "unknown"}</dd>
        </div>
        <div>
          <dt>Region</dt>
          <dd>{sandbox.region ?? "automatic"}</dd>
        </div>
      </dl>
      <CopyCommand
        command={sandboxSshCommand(sandbox.name)}
        label="SSH command for this sandbox"
      />
    </li>
  );
}

export function RunObservatory({
  initialSnapshot
}: {
  readonly initialSnapshot: ControlPlaneSnapshot;
}) {
  const [snapshot, setSnapshot] = useState(initialSnapshot);
  const [trigger, setTrigger] = useState("all");
  const [refreshing, setRefreshing] = useState(false);
  const refreshController = useRef<AbortController | null>(null);

  async function refresh() {
    if (refreshController.current) return;
    const controller = new AbortController();
    refreshController.current = controller;
    setRefreshing(true);
    try {
      const response = await fetch("/api/control-plane", {
        cache: "no-store",
        signal: controller.signal
      });
      if (!response.ok) throw new Error("Could not refresh runs.");
      const next = (await response.json()) as ControlPlaneSnapshot;
      startTransition(() => setSnapshot(next));
    } catch (error) {
      if (!(error instanceof DOMException && error.name === "AbortError")) {
        startTransition(() =>
          setSnapshot((current) => ({
            ...current,
            error:
              error instanceof Error ? error.message : "Could not refresh runs."
          }))
        );
      }
    } finally {
      refreshController.current = null;
      setRefreshing(false);
    }
  }
  const poll = useEffectEvent(refresh);

  useEffect(() => {
    const timer = window.setInterval(() => {
      if (document.visibilityState === "visible") void poll();
    }, POLL_INTERVAL_MS);
    return () => {
      refreshController.current?.abort();
      window.clearInterval(timer);
    };
  }, []);

  const visibleRuns = snapshot.runs.filter(
    (run) => trigger === "all" || run.trigger === trigger
  );
  const triggers = Array.from(
    new Set(snapshot.runs.map((run) => run.trigger))
  ).sort();

  return (
    <section className="observatory" aria-label="Factory activity">
      {!snapshot.configured ? (
        <div className="registryNotice" role="status">
          Connect a private Vercel Blob store to enable durable run history.
        </div>
      ) : null}
      {snapshot.error ? (
        <div className="registryNotice registryNotice-error" role="alert">
          {snapshot.error}
        </div>
      ) : null}

      <div className="runToolbar">
        <div
          className="sourceFilters"
          role="group"
          aria-label="Filter tickets by trigger"
        >
          {["all", ...triggers].map((value) => (
            <Button
              aria-pressed={trigger === value}
              className="filterButton"
              key={value}
              onClick={() => setTrigger(value)}
              size="sm"
              type="button"
              variant="ghost"
            >
              {value === "all" ? "All sources" : value}
            </Button>
          ))}
        </div>
        <Button
          className="refreshButton"
          disabled={refreshing}
          onClick={() => void refresh()}
          size="sm"
          type="button"
          variant="outline"
        >
          {refreshing ? "Refreshing…" : "Refresh"}
        </Button>
      </div>

      <div className="runBoardViewport">
        <div className="runBoard">
          {BOARD_COLUMNS.map((column) => {
            const runs = visibleRuns.filter((run) =>
              column.statuses.some((status) => status === run.status)
            );
            return (
              <section
                className="runColumn"
                aria-labelledby={`column-${column.key}`}
                key={column.key}
              >
                <header className="runColumnHeader">
                  <h3 id={`column-${column.key}`}>{column.title}</h3>
                  <span aria-label={`${runs.length} tickets`}>
                    {runs.length}
                  </span>
                </header>
                {runs.length > 0 ? (
                  <ol className="runList">
                    {runs.map((run) => (
                      <RunTicket key={run.id} run={run} />
                    ))}
                  </ol>
                ) : (
                  <p className="emptyRunway">{column.empty}</p>
                )}
              </section>
            );
          })}
        </div>
      </div>

      <div className="sandboxInventoryHeader">
        <h3>Sandbox inventory</h3>
        <span>{snapshot.sandboxes.length} resources</span>
      </div>
      {snapshot.sandboxError ? (
        <p className="emptySandboxes">
          Sandbox inventory is currently unavailable.
        </p>
      ) : snapshot.sandboxes.length > 0 ? (
        <ul className="sandboxGrid">
          {snapshot.sandboxes.slice(0, 8).map((sandbox) => (
            <SandboxCard key={sandbox.name} sandbox={sandbox} />
          ))}
        </ul>
      ) : (
        <p className="emptySandboxes">No named sandbox resources are active.</p>
      )}
    </section>
  );
}
