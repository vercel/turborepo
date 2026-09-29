"use client";

import { useState } from "react";

import type { OperatorRunAction } from "../agent/lib/operator-runs";

type RunState = "starting" | "running" | "done" | "error";

interface RunModels {
  readonly authorModel: string;
  readonly reviewerModel: string;
}

export interface RunStatus {
  readonly cursor?: number;
  readonly error?: string;
  readonly models?: RunModels;
  readonly sessionId?: string;
  readonly state: RunState;
  readonly statusPath?: string;
  readonly workspaceId?: string;
}

const POLL_INTERVAL_MS = 3000;
const RUN_START_TIMEOUT_MS = 30_000;

function isRunModels(value: unknown): value is RunModels {
  if (typeof value !== "object" || value === null) {
    return false;
  }

  const candidate = value as Record<string, unknown>;
  return (
    typeof candidate.authorModel === "string" &&
    typeof candidate.reviewerModel === "string"
  );
}

function isRunStatus(value: unknown): value is RunStatus {
  if (typeof value !== "object" || value === null) {
    return false;
  }

  const candidate = value as Record<string, unknown>;
  return (
    (candidate.state === "running" ||
      candidate.state === "done" ||
      candidate.state === "error") &&
    (candidate.sessionId === undefined ||
      typeof candidate.sessionId === "string") &&
    (candidate.models === undefined || isRunModels(candidate.models)) &&
    (candidate.statusPath === undefined ||
      typeof candidate.statusPath === "string") &&
    (candidate.workspaceId === undefined ||
      typeof candidate.workspaceId === "string") &&
    (candidate.cursor === undefined ||
      (typeof candidate.cursor === "number" &&
        Number.isInteger(candidate.cursor) &&
        candidate.cursor >= 0))
  );
}

export function runLabel(status: RunStatus | null, idleLabel: string): string {
  return status?.state === "starting"
    ? "Starting…"
    : status?.state === "running"
      ? "Running…"
      : idleLabel;
}

export function useOperatorRun(
  action: OperatorRunAction,
  startPath = "/eve/v1/operator/runs"
) {
  const [status, setStatus] = useState<RunStatus | null>(null);

  async function start(body: Record<string, string> = {}) {
    setStatus({ state: "starting" });

    try {
      const response = await fetch(startPath, {
        body: JSON.stringify(body),
        headers: {
          "content-type": "application/json",
          "x-operator-action": action
        },
        method: "POST",
        signal: AbortSignal.timeout(RUN_START_TIMEOUT_MS)
      });

      if (!response.ok) {
        throw new Error("Could not start the run.");
      }
      const initialStatus: unknown = await response.json();
      if (!isRunStatus(initialStatus) || !initialStatus.sessionId) {
        throw new Error("The run returned an invalid session.");
      }
      setStatus(initialStatus);
      await pollRun(initialStatus.sessionId, initialStatus.statusPath);
    } catch (error) {
      setStatus((current) => ({
        ...current,
        error:
          error instanceof DOMException && error.name === "TimeoutError"
            ? "The run is still retrying in Eve. Check Agent Runs before starting another."
            : error instanceof Error
              ? error.message
              : "Could not start the run.",
        state: "error"
      }));
    }
  }

  async function pollRun(sessionId: string, statusPath?: string) {
    let cursor = 0;
    while (true) {
      await new Promise((resolve) => setTimeout(resolve, POLL_INTERVAL_MS));
      const query = new URLSearchParams({ cursor: String(cursor) });
      const response = await fetch(
        statusPath ??
          `/eve/v1/operator/runs/${encodeURIComponent(sessionId)}/status?${query}`,
        { cache: "no-store" }
      );
      if (!response.ok) {
        throw new Error("Could not read the run status.");
      }
      const nextStatus: unknown = await response.json();
      if (!isRunStatus(nextStatus)) {
        throw new Error("The run returned an invalid status.");
      }
      cursor = nextStatus.cursor ?? cursor;
      // The status route reports progress only, so keep the models the start
      // response resolved for this session.
      setStatus((current) => ({ ...current, ...nextStatus }));
      if (nextStatus.state !== "running") return;
    }
  }

  return {
    isBusy: status?.state === "starting" || status?.state === "running",
    start,
    status
  };
}

interface RunStatusPanelProps {
  readonly status: RunStatus | null;
}

export function RunStatusPanel({ status }: RunStatusPanelProps) {
  if (!status) {
    return null;
  }

  return (
    <div
      className={`mt-6 flex items-start gap-3 rounded-md bg-muted p-3.5 text-[0.8125rem] ${status.state === "starting" || status.state === "running" ? "text-warning" : status.state === "done" ? "text-success" : "text-destructive"}`}
      role={status.state === "error" ? "alert" : "status"}
      aria-live={status.state === "error" ? "assertive" : "polite"}
    >
      <span
        className="mt-1.5 size-[7px] shrink-0 rounded-full bg-current"
        aria-hidden="true"
      />
      <div>
        <strong className="block font-semibold capitalize">
          {status.state}
        </strong>
        {status.error ? (
          <span className="mt-1 block text-xs text-muted-foreground">
            {status.error}
          </span>
        ) : null}
        {status.sessionId ? (
          <code className="mt-1 block wrap-anywhere font-mono text-xs text-muted-foreground">
            {status.sessionId}
          </code>
        ) : null}
        {status.models ? (
          <>
            <code className="mt-1 block wrap-anywhere font-mono text-xs text-muted-foreground">
              author {status.models.authorModel}
            </code>
            <code className="mt-1 block wrap-anywhere font-mono text-xs text-muted-foreground">
              reviewer {status.models.reviewerModel}
            </code>
          </>
        ) : null}
        {status.workspaceId ? (
          <a
            className="mt-2 block text-xs font-medium text-foreground hover:underline hover:underline-offset-4"
            href={`/workspaces/${encodeURIComponent(status.workspaceId)}`}
          >
            View live conversation →
          </a>
        ) : null}
      </div>
    </div>
  );
}
