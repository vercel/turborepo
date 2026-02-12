"use client";

import { useEffect, useState } from "react";
import Link from "next/link";
import type { RunMeta } from "@/lib/runs";

const STATUS_LABELS: Record<string, { label: string; color: string }> = {
  queued: { label: "Queued", color: "text-neutral-400" },
  scanning: { label: "Scanning", color: "text-blue-400" },
  fixing: { label: "Fixing", color: "text-yellow-400" },
  pushing: { label: "Pushing", color: "text-purple-400" },
  completed: { label: "Completed", color: "text-green-400" },
  failed: { label: "Failed", color: "text-red-400" }
};

function StatusBadge({ status }: { status: string }) {
  const config = STATUS_LABELS[status] ?? {
    label: status,
    color: "text-neutral-400"
  };
  const isActive = ["queued", "scanning", "fixing", "pushing"].includes(status);

  return (
    <span
      className={`inline-flex items-center gap-1.5 text-xs font-medium ${config.color}`}
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

function timeAgo(dateStr: string): string {
  const seconds = Math.floor((Date.now() - new Date(dateStr).getTime()) / 1000);
  if (seconds < 60) return `${seconds}s ago`;
  const minutes = Math.floor(seconds / 60);
  if (minutes < 60) return `${minutes}m ago`;
  const hours = Math.floor(minutes / 60);
  if (hours < 24) return `${hours}h ago`;
  const days = Math.floor(hours / 24);
  return `${days}d ago`;
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

export default function DashboardPage() {
  const [runs, setRuns] = useState<RunMeta[]>([]);
  const [loading, setLoading] = useState(true);

  useEffect(() => {
    let active = true;

    async function fetchRuns() {
      try {
        const res = await fetch("/api/runs");
        if (res.ok && active) {
          setRuns(await res.json());
        }
      } catch {
        // Silently retry on next interval
      } finally {
        if (active) setLoading(false);
      }
    }

    fetchRuns();
    const interval = setInterval(fetchRuns, 5_000);
    return () => {
      active = false;
      clearInterval(interval);
    };
  }, []);

  const activeRuns = runs.filter((r) =>
    ["queued", "scanning", "fixing", "pushing"].includes(r.status)
  );
  const pastRuns = runs.filter(
    (r) => r.status === "completed" || r.status === "failed"
  );

  return (
    <main className="mx-auto max-w-4xl px-6 py-16 font-mono">
      <div className="mb-8 flex items-center justify-between">
        <div>
          <h1 className="text-2xl font-bold">Dashboard</h1>
          <p className="text-sm text-neutral-500">
            Agent runs and sandbox logs
          </p>
        </div>
        <Link
          href="/"
          className="rounded border border-neutral-300 px-3 py-1.5 text-sm text-neutral-700 hover:bg-neutral-100 dark:border-neutral-800 dark:text-neutral-200 dark:hover:bg-neutral-800"
        >
          Home
        </Link>
      </div>

      {loading && (
        <div className="py-12 text-center text-neutral-500">
          Loading runs...
        </div>
      )}

      {!loading && runs.length === 0 && (
        <div className="rounded border border-neutral-800 p-8 text-center text-neutral-500">
          No runs yet. Trigger an audit from the{" "}
          <Link href="/" className="underline hover:text-white">
            home page
          </Link>
          .
        </div>
      )}

      {activeRuns.length > 0 && (
        <section className="mb-10">
          <h2 className="mb-3 text-sm font-semibold uppercase tracking-wider text-neutral-500">
            Active
          </h2>
          <div className="space-y-2">
            {activeRuns.map((run) => (
              <RunCard key={run.id} run={run} />
            ))}
          </div>
        </section>
      )}

      {pastRuns.length > 0 && (
        <section>
          <h2 className="mb-3 text-sm font-semibold uppercase tracking-wider text-neutral-500">
            History
          </h2>
          <div className="space-y-2">
            {pastRuns.map((run) => (
              <RunCard key={run.id} run={run} />
            ))}
          </div>
        </section>
      )}
    </main>
  );
}

function RunCard({ run }: { run: RunMeta }) {
  const vulnCount = run.vulnerabilities
    ? run.vulnerabilities.cargo + run.vulnerabilities.pnpm
    : null;

  return (
    <Link
      href={`/dashboard/${run.id}`}
      className="flex items-center justify-between rounded border border-neutral-800 p-4 transition-colors hover:border-neutral-600 hover:bg-neutral-900/50"
    >
      <div className="flex items-center gap-4">
        <StatusBadge status={run.status} />
        <div>
          <p className="text-sm font-medium">{run.id}</p>
          <p className="text-xs text-neutral-500">
            {run.trigger === "cron" ? "Scheduled" : "Manual"} ·{" "}
            {timeAgo(run.createdAt)}
            {run.status === "completed" || run.status === "failed"
              ? ` · ${duration(run.createdAt, run.updatedAt)}`
              : ""}
          </p>
        </div>
      </div>
      <div className="flex items-center gap-4 text-xs text-neutral-500">
        {vulnCount !== null && <span>{vulnCount} vulns</span>}
        {run.agentResults && (
          <span>{run.agentResults.vulnerabilitiesFixed} fixed</span>
        )}
        {run.branch && (
          <span className="max-w-[200px] truncate" title={run.branch}>
            {run.branch}
          </span>
        )}
        <span className="text-neutral-700">&rarr;</span>
      </div>
    </Link>
  );
}
