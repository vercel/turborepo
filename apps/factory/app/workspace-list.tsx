"use client";

import Link from "next/link";
import { useCallback, useEffect, useState } from "react";

import { Button } from "../components/ui/button";
import type { WorkspaceSummary } from "./workspace-types";
import { isWorkspaceRunning, workspaceStatusLabel } from "./workspace-types";

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
      const response = await fetch("/api/workspaces", { cache: "no-store" });
      if (!response.ok)
        throw new Error(`Could not load workspaces (${response.status}).`);
      const result = (await response.json()) as {
        readonly workspaces: readonly WorkspaceSummary[];
      };
      setWorkspaces(result.workspaces);
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
            <li key={workspace.id}>
              <Link
                className="grid grid-cols-[minmax(0,1fr)_auto] gap-4 rounded-md border border-border p-4 text-foreground no-underline hover:bg-accent focus-visible:outline-2 focus-visible:outline-ring focus-visible:outline-offset-2 max-[520px]:grid-cols-1"
                href={`/workspaces/${encodeURIComponent(workspace.id)}`}
              >
                <span className="min-w-0">
                  <strong className="block truncate text-sm font-semibold">
                    {workspace.title || "Untitled workspace"}
                  </strong>
                  <span className="mt-1 block truncate font-mono text-xs text-muted-foreground">
                    {workspace.id}
                  </span>
                </span>
                <span className="flex items-center gap-5 text-xs text-muted-foreground max-[520px]:justify-between">
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
                    {workspaceStatusLabel(workspace.status)}
                  </span>
                </span>
              </Link>
            </li>
          ))}
        </ol>
      )}
    </div>
  );
}
