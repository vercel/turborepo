"use client";

import { useEffect, useRef, useState, use } from "react";
import Link from "next/link";
import type { RunMeta } from "@/lib/runs";

const STATUS_LABELS: Record<string, { label: string; color: string }> = {
  queued: { label: "Queued", color: "text-neutral-400" },
  scanning: { label: "Scanning", color: "text-blue-400" },
  fixing: { label: "Fixing", color: "text-yellow-400" },
  completed: { label: "Completed", color: "text-green-400" },
  failed: { label: "Failed", color: "text-red-400" }
};

function StatusBadge({ status }: { status: string }) {
  const config = STATUS_LABELS[status] ?? {
    label: status,
    color: "text-neutral-400"
  };
  const isActive = ["queued", "scanning", "fixing"].includes(status);

  return (
    <span
      className={`inline-flex items-center gap-1.5 text-sm font-medium ${config.color}`}
    >
      {isActive && (
        <span className="relative flex h-2 w-2">
          <span
            className={`absolute inline-flex h-full w-full animate-ping rounded-full opacity-75 ${config.color.replace("text-", "bg-")}`}
          />
          <span
            className={`relative inline-flex h-2 w-2 rounded-full ${config.color.replace("text-", "bg-")}`}
          />
        </span>
      )}
      {!isActive && (
        <span
          className={`inline-flex h-2 w-2 rounded-full ${config.color.replace("text-", "bg-")}`}
        />
      )}
      {config.label}
    </span>
  );
}

function duration(start: string, end: string): string {
  const ms = new Date(end).getTime() - new Date(start).getTime();
  const seconds = Math.floor(ms / 1000);
  if (seconds < 60) return `${seconds}s`;
  const minutes = Math.floor(seconds / 60);
  const remainingSeconds = seconds % 60;
  if (minutes < 60) return `${minutes}m ${remainingSeconds}s`;
  const hours = Math.floor(minutes / 60);
  const remainingMinutes = minutes % 60;
  return `${hours}h ${remainingMinutes}m`;
}

function CopyApplyButton({ diffUrl }: { diffUrl: string }) {
  const [copied, setCopied] = useState(false);

  function handleCopy() {
    const proxyPath = `/api/blob?url=${encodeURIComponent(diffUrl)}`;
    const absolute =
      typeof window !== "undefined"
        ? `${window.location.origin}${proxyPath}`
        : proxyPath;
    const command = `curl -sL '${absolute}' | git apply`;
    navigator.clipboard.writeText(command);
    setCopied(true);
    setTimeout(() => setCopied(false), 2000);
  }

  return (
    <button
      onClick={handleCopy}
      className="rounded border border-neutral-800 px-3 py-1.5 text-xs hover:bg-neutral-800"
    >
      {copied ? "Copied!" : "Copy apply command"}
    </button>
  );
}

export default function RunDetailPage({
  params
}: {
  params: Promise<{ id: string }>;
}) {
  const { id } = use(params);
  const [run, setRun] = useState<RunMeta | null>(null);
  const [logs, setLogs] = useState("");
  const [autoScroll, setAutoScroll] = useState(true);
  const [loading, setLoading] = useState(true);
  const logEndRef = useRef<HTMLDivElement>(null);
  const logContainerRef = useRef<HTMLPreElement>(null);

  const isActive = run
    ? ["queued", "scanning", "fixing"].includes(run.status)
    : false;

  // Fetch run metadata
  useEffect(() => {
    let active = true;

    async function fetchRun() {
      try {
        const res = await fetch(`/api/runs/${id}`);
        if (res.ok && active) {
          setRun(await res.json());
        }
      } catch {
        // Retry on next interval
      } finally {
        if (active) setLoading(false);
      }
    }

    fetchRun();
    const interval = setInterval(fetchRun, 3_000);
    return () => {
      active = false;
      clearInterval(interval);
    };
  }, [id]);

  // Fetch logs (polls while run is active)
  useEffect(() => {
    let active = true;

    async function fetchLogs() {
      try {
        const res = await fetch(`/api/runs/${id}/logs`);
        if (res.ok && active) {
          setLogs(await res.text());
        }
      } catch {
        // Retry on next interval
      }
    }

    fetchLogs();
    // Poll faster while active, slower when done
    const interval = setInterval(fetchLogs, isActive ? 3_000 : 10_000);
    return () => {
      active = false;
      clearInterval(interval);
    };
  }, [id, isActive]);

  // Auto-scroll to bottom when new logs arrive
  useEffect(() => {
    if (autoScroll && logEndRef.current) {
      logEndRef.current.scrollIntoView({ behavior: "smooth" });
    }
  }, [logs, autoScroll]);

  // Detect manual scroll to pause auto-scroll
  function handleScroll() {
    const container = logContainerRef.current;
    if (!container) return;
    const isAtBottom =
      container.scrollHeight - container.scrollTop - container.clientHeight <
      50;
    setAutoScroll(isAtBottom);
  }

  if (loading) {
    return (
      <main className="mx-auto max-w-5xl px-6 py-16 font-mono">
        <div className="py-12 text-center text-neutral-500">Loading...</div>
      </main>
    );
  }

  if (!run) {
    return (
      <main className="mx-auto max-w-5xl px-6 py-16 font-mono">
        <div className="py-12 text-center text-neutral-500">Run not found.</div>
      </main>
    );
  }

  return (
    <main className="mx-auto max-w-5xl px-6 py-16 font-mono">
      <div className="mb-6 flex items-center justify-between">
        <div className="flex items-center gap-4">
          <Link
            href="/dashboard"
            className="text-sm text-neutral-500 hover:text-white"
          >
            &larr; Dashboard
          </Link>
          <StatusBadge status={run.status} />
        </div>
      </div>

      <h1 className="mb-6 text-xl font-bold">{run.id}</h1>

      {/* Metadata grid */}
      <div className="mb-8 grid grid-cols-2 gap-4 rounded border border-neutral-800 p-4 text-sm md:grid-cols-4">
        <div>
          <p className="text-xs text-neutral-500">Trigger</p>
          <p>{run.trigger === "cron" ? "Scheduled" : "Manual"}</p>
        </div>
        <div>
          <p className="text-xs text-neutral-500">Started</p>
          <p>{new Date(run.createdAt).toLocaleString()}</p>
        </div>
        <div>
          <p className="text-xs text-neutral-500">Duration</p>
          <p>
            {run.status === "completed" || run.status === "failed"
              ? duration(run.createdAt, run.updatedAt)
              : duration(run.createdAt, new Date().toISOString())}
            {isActive && "..."}
          </p>
        </div>
        <div>
          <p className="text-xs text-neutral-500">Sandbox</p>
          <p className="truncate" title={run.sandboxId}>
            {run.sandboxId ?? "--"}
          </p>
        </div>

        {run.vulnerabilities && (
          <>
            <div>
              <p className="text-xs text-neutral-500">Cargo vulns</p>
              <p>{run.vulnerabilities.cargo}</p>
            </div>
            <div>
              <p className="text-xs text-neutral-500">pnpm vulns</p>
              <p>{run.vulnerabilities.pnpm}</p>
            </div>
          </>
        )}

        {run.agentResults && (
          <>
            <div>
              <p className="text-xs text-neutral-500">Fixed</p>
              <p className="text-green-400">
                {run.agentResults.vulnerabilitiesFixed}
              </p>
            </div>
            <div>
              <p className="text-xs text-neutral-500">Remaining</p>
              <p
                className={
                  run.agentResults.vulnerabilitiesRemaining > 0
                    ? "text-yellow-400"
                    : "text-green-400"
                }
              >
                {run.agentResults.vulnerabilitiesRemaining}
              </p>
            </div>
          </>
        )}

        {run.branch && (
          <div className="col-span-2">
            <p className="text-xs text-neutral-500">Branch</p>
            <p className="truncate" title={run.branch}>
              {run.branch}
            </p>
          </div>
        )}

        {run.error && (
          <div className="col-span-full">
            <p className="text-xs text-neutral-500">Error</p>
            <p className="text-red-400">{run.error}</p>
          </div>
        )}

        {run.agentResults?.summary && (
          <div className="col-span-full">
            <p className="text-xs text-neutral-500">Summary</p>
            <p>{run.agentResults.summary}</p>
          </div>
        )}
      </div>

      {/* Links */}
      <div className="mb-6 flex gap-3">
        {run.diffUrl && (
          <>
            <Link
              href={`/vuln-diffs/view?url=${encodeURIComponent(run.diffUrl)}`}
              className="rounded border border-neutral-800 px-3 py-1.5 text-xs hover:bg-neutral-800"
            >
              View diff
            </Link>
            <CopyApplyButton diffUrl={run.diffUrl} />
          </>
        )}
      </div>

      {/* Logs */}
      <div className="mb-2 flex items-center justify-between">
        <h2 className="text-sm font-semibold uppercase tracking-wider text-neutral-500">
          Logs
        </h2>
        <div className="flex items-center gap-3 text-xs text-neutral-500">
          {isActive && <span className="text-blue-400">Streaming...</span>}
          <button
            onClick={() => setAutoScroll(!autoScroll)}
            className={`rounded px-2 py-1 ${autoScroll ? "bg-neutral-800 text-white" : "text-neutral-500 hover:text-white"}`}
          >
            Auto-scroll {autoScroll ? "on" : "off"}
          </button>
        </div>
      </div>
      <pre
        ref={logContainerRef}
        onScroll={handleScroll}
        className="h-[500px] overflow-auto rounded border border-neutral-800 bg-neutral-950 p-4 text-xs leading-5 text-neutral-300"
      >
        {logs || (
          <span className="text-neutral-600">
            {isActive
              ? "Waiting for logs..."
              : "No logs recorded for this run."}
          </span>
        )}
        <div ref={logEndRef} />
      </pre>
    </main>
  );
}
