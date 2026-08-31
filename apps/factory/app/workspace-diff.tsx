"use client";

import type { FileDiffOptions } from "@pierre/diffs";
import { PatchDiff } from "@pierre/diffs/react";
import { Loader2Icon, RefreshCwIcon } from "lucide-react";
import { useCallback, useEffect, useMemo, useState } from "react";

interface WorkspaceDiffProps {
  readonly busy: boolean;
  readonly workspaceId: string;
}

interface WorkspaceDiffResponse {
  readonly error?: string;
  readonly patch?: string;
}

export function WorkspaceDiff({ busy, workspaceId }: WorkspaceDiffProps) {
  const [patch, setPatch] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [loading, setLoading] = useState(true);
  const options = useMemo<FileDiffOptions<undefined>>(
    () => ({
      diffStyle: "unified",
      overflow: "scroll",
      theme: { dark: "pierre-dark", light: "pierre-light" }
    }),
    []
  );

  const refresh = useCallback(async () => {
    setLoading(true);
    setError(null);
    try {
      const response = await fetch(
        `/api/workspaces/${encodeURIComponent(workspaceId)}/diff`,
        { cache: "no-store" }
      );
      const body = (await response
        .json()
        .catch(() => ({}))) as WorkspaceDiffResponse;
      if (!response.ok) {
        throw new Error(
          body.error ?? `Could not load workspace diff (${response.status}).`
        );
      }
      setPatch(body.patch ?? "");
    } catch (cause) {
      setError(
        cause instanceof Error
          ? cause.message
          : "Could not load workspace diff."
      );
    } finally {
      setLoading(false);
    }
  }, [workspaceId]);

  useEffect(() => {
    void refresh();
    if (!busy) return;
    const interval = window.setInterval(() => void refresh(), 3000);
    return () => window.clearInterval(interval);
  }, [busy, refresh]);

  return (
    <section
      aria-labelledby="workspace-diff-tab"
      className="min-h-0 flex-1 overflow-y-auto bg-muted/20 px-6 py-6 max-[520px]:px-4"
      id="workspace-diff-panel"
      role="tabpanel"
    >
      <div className="mx-auto flex w-full max-w-5xl items-center justify-between gap-4 pb-4">
        <div>
          <h2 className="text-sm font-medium">Git diff</h2>
          <p className="mt-0.5 text-xs text-muted-foreground">
            Changes against <code>HEAD</code>
          </p>
        </div>
        <button
          className="inline-flex items-center gap-1.5 rounded-md border border-border bg-background px-2.5 py-1.5 text-xs text-muted-foreground transition-colors hover:text-foreground disabled:opacity-50"
          disabled={loading}
          onClick={() => void refresh()}
          type="button"
        >
          {loading ? (
            <Loader2Icon className="size-3 animate-spin" />
          ) : (
            <RefreshCwIcon className="size-3" />
          )}
          Refresh
        </button>
      </div>

      <div className="mx-auto w-full max-w-5xl">
        {error ? (
          <div
            className="rounded-md border border-destructive/20 bg-destructive/10 px-4 py-3 text-sm text-destructive"
            role="alert"
          >
            {error}
          </div>
        ) : patch ? (
          <PatchDiff options={options} patch={patch} />
        ) : loading ? (
          <div
            className="grid min-h-64 place-items-center rounded-md border border-border bg-background text-sm text-muted-foreground"
            role="status"
          >
            <span className="inline-flex items-center gap-2">
              <Loader2Icon className="size-4 animate-spin" />
              Loading git diff…
            </span>
          </div>
        ) : (
          <div className="grid min-h-64 place-items-center rounded-md border border-border bg-background px-4 text-center">
            <div>
              <p className="text-sm font-medium">No changes yet</p>
              <p className="mt-1 text-xs text-muted-foreground">
                The workspace matches <code>HEAD</code>.
              </p>
            </div>
          </div>
        )}
      </div>
    </section>
  );
}
