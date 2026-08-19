"use client";

import {
  startTransition,
  useEffect,
  useEffectEvent,
  useRef,
  useState
} from "react";

import type {
  AgentRunRecord,
  ControlPlaneSnapshot,
  SandboxResource
} from "../agent/lib/run-types";

const AGENT_RUNS_URL =
  "https://vercel.com/vercel-internal-apps/turborepo-eve-agent/observability/agent-runs";
const WORKFLOW_RUNS_URL =
  "https://vercel.com/vercel-internal-apps/turborepo-eve-agent/observability/workflows";
const POLL_INTERVAL_MS = 15_000;

type SourceFilter = "all" | AgentRunRecord["source"];

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

function RunFlightStrip({ run }: { readonly run: AgentRunRecord }) {
  const detailsUrl = run.source === "eve" ? AGENT_RUNS_URL : WORKFLOW_RUNS_URL;
  return (
    <li>
      <article className="runStrip">
        <div className={`runState runState-${run.status}`}>
          <span className="statusDot" aria-hidden="true" />
          <span>{run.status}</span>
        </div>
        <div className="runIdentity">
          <strong>{run.title}</strong>
          <code>{run.id}</code>
        </div>
        <dl className="runMetadata">
          <div>
            <dt>Runtime</dt>
            <dd>{run.harness ?? run.agent}</dd>
          </div>
          <div>
            <dt>Trigger</dt>
            <dd>{run.trigger}</dd>
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
          <div>
            <dt>Duration</dt>
            <dd>{duration(run)}</dd>
          </div>
        </dl>
        <div className="sandboxTrack">
          <span>{run.source}</span>
          <span aria-hidden="true">→</span>
          <span>{run.sandbox?.provider ?? "no sandbox"}</span>
          <span aria-hidden="true">→</span>
          <span>{run.sandbox?.status ?? run.status}</span>
        </div>
        <a
          className="runDetails"
          href={detailsUrl}
          rel="noreferrer"
          target="_blank"
        >
          Inspect <span aria-hidden="true">↗</span>
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
    </li>
  );
}

export function RunObservatory({
  initialSnapshot
}: {
  readonly initialSnapshot: ControlPlaneSnapshot;
}) {
  const [snapshot, setSnapshot] = useState(initialSnapshot);
  const [source, setSource] = useState<SourceFilter>("all");
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
    (run) => source === "all" || run.source === source
  );
  const active = snapshot.runs.filter(
    (run) => run.status === "running" || run.status === "waiting"
  ).length;
  const failed = snapshot.runs.filter((run) => run.status === "failed").length;
  const completed = snapshot.runs.filter(
    (run) => run.status === "completed"
  ).length;

  return (
    <section className="observatory" aria-labelledby="runs-title">
      <div className="observatoryHeader">
        <div>
          <p className="eyebrow">Unified telemetry</p>
          <h2 id="runs-title">Agent runway</h2>
          <p>
            Eve and Harness sessions, their execution state, and the sandboxes
            underneath them.
          </p>
        </div>
        <div className="runCounters" aria-label="Run totals">
          <span>
            <strong>{active}</strong> active
          </span>
          <span>
            <strong>{completed}</strong> complete
          </span>
          <span>
            <strong>{failed}</strong> failed
          </span>
        </div>
      </div>

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
        <div className="sourceFilters" aria-label="Filter runs by source">
          {(["all", "eve", "harness"] as const).map((value) => (
            <button
              aria-pressed={source === value}
              className="filterButton"
              key={value}
              onClick={() => setSource(value)}
              type="button"
            >
              {value}
            </button>
          ))}
        </div>
        <button
          className="refreshButton"
          disabled={refreshing}
          onClick={() => void refresh()}
          type="button"
        >
          {refreshing ? "Refreshing…" : "Refresh"}
        </button>
      </div>

      {visibleRuns.length > 0 ? (
        <ol className="runList">
          {visibleRuns.map((run) => (
            <RunFlightStrip key={run.id} run={run} />
          ))}
        </ol>
      ) : (
        <div className="emptyRunway">
          No {source === "all" ? "" : `${source} `}runs recorded yet.
        </div>
      )}

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
