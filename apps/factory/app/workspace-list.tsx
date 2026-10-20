"use client";

import Link from "next/link";
import { useCallback, useEffect, useState } from "react";

import { Button } from "../components/ui/button";
import {
  isWorkspaceRunning,
  workspaceStatusLabel,
  type WorkspaceDisplayStatus,
  type WorkspaceSummary
} from "./workspace-status-types";

const POLL_INTERVAL_MS = 10_000;

function formatTimestamp(value: string): string {
  return new Intl.DateTimeFormat("en-US", {
    dateStyle: "medium",
    timeStyle: "short"
  }).format(new Date(value));
}

export function WorkspaceList() {
  const [workspaces, setWorkspaces] = useState<readonly WorkspaceSummary[]>([]);
  const [error, setError] = useState<string | null>(null);
  const [loading, setLoading] = useState(true);

  const refresh = useCallback(async () => {
    try {
      const [response, statusResponse] = await Promise.all([
        fetch("/api/workspaces", { cache: "no-store" }),
        fetch("/api/workspace-statuses", { cache: "no-store" })
      ]);
      if (!response.ok || !statusResponse.ok)
        throw new Error("Could not load workspace statuses.");
      const result = (await response.json()) as {
        readonly workspaces: readonly WorkspaceSummary[];
      };
      const statusResult = (await statusResponse.json()) as {
        readonly statuses: Readonly<Record<string, WorkspaceDisplayStatus>>;
      };
      setWorkspaces(
        result.workspaces.map((workspace) => ({
          ...workspace,
          ...statusResult.statuses[workspace.id]
        }))
      );
      setError(null);
    } catch (cause) {
      setError(
        cause instanceof Error ? cause.message : "Could not load workspaces."
      );
    } finally {
      setLoading(false);
    }
  }, []);

  useEffect(() => {
    void refresh();
    const onVisibilityChange = () => {
      if (document.visibilityState === "visible") void refresh();
    };
    const timer = window.setInterval(() => {
      if (document.visibilityState === "visible") void refresh();
    }, POLL_INTERVAL_MS);
    document.addEventListener("visibilitychange", onVisibilityChange);
    return () => {
      document.removeEventListener("visibilitychange", onVisibilityChange);
      window.clearInterval(timer);
    };
  }, [refresh]);

  if (loading) {
    return (
      <p className="py-12 text-sm text-muted-foreground" role="status">
        Loading workspaces…
      </p>
    );
  }

  return (
    <div>
      {error ? (
        <div
          className="mb-5 flex items-center justify-between gap-4 rounded-md border border-destructive p-3 text-sm"
          role="alert"
        >
          <span>{error}</span>
          <Button
            onClick={() => void refresh()}
            size="sm"
            type="button"
            variant="outline"
          >
            Retry
          </Button>
        </div>
      ) : null}
      {workspaces.length === 0 ? (
        <p className="grid min-h-44 place-items-center rounded-md border border-dashed border-border p-6 text-center text-sm text-muted-foreground">
          No workspaces yet. Start work to create one.
        </p>
      ) : (
        <ol className="grid list-none gap-3 p-0">
          {workspaces.map((workspace) => (
            <li
              className="grid grid-cols-[minmax(0,1fr)_auto] gap-4 rounded-md border border-border p-4 hover:bg-accent max-[520px]:grid-cols-1"
              key={workspace.id}
            >
              <Link
                className="min-w-0 text-foreground no-underline focus-visible:outline-2 focus-visible:outline-ring focus-visible:outline-offset-2"
                href={`/workspaces/${encodeURIComponent(workspace.id)}`}
              >
                <strong className="block truncate text-sm font-semibold">
                  {workspace.title || "Untitled workspace"}
                </strong>
                <span className="mt-1 block truncate font-mono text-xs text-muted-foreground">
                  {workspace.id}
                </span>
              </Link>
              <span className="flex items-center gap-5 text-xs text-muted-foreground max-[520px]:justify-between">
                {workspace.pullRequest ? (
                  <a
                    className="font-medium text-foreground hover:underline hover:underline-offset-4 focus-visible:outline-2 focus-visible:outline-ring focus-visible:outline-offset-2"
                    href={workspace.pullRequest.url}
                    rel="noreferrer"
                    target="_blank"
                  >
                    PR #{workspace.pullRequest.number}{" "}
                    <span className="sr-only">in a new tab</span>
                    <span aria-hidden="true">↗</span>
                  </a>
                ) : null}
                <time dateTime={workspace.updatedAt}>
                  {formatTimestamp(workspace.updatedAt)}
                </time>
                <span
                  className={`flex items-center gap-2 ${isWorkspaceRunning(workspace.status) ? "text-warning" : workspace.status === "error" ? "text-destructive" : "text-success"}`}
                >
                  <span
                    className="size-1.5 rounded-full bg-current"
                    aria-hidden="true"
                  />
                  {workspaceStatusLabel(workspace)}
                </span>
              </span>
            </li>
          ))}
        </ol>
      )}
    </div>
  );
}
