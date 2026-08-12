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
  readonly models?: RunModels;
  readonly sessionId?: string;
  readonly state: RunState;
}

const POLL_INTERVAL_MS = 3000;

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

export function useOperatorRun(action: OperatorRunAction) {
  const [status, setStatus] = useState<RunStatus | null>(null);

  async function start(body: Record<string, string> = {}) {
    setStatus({ state: "starting" });

    try {
      const response = await fetch("/eve/v1/operator/runs", {
        body: JSON.stringify(body),
        headers: {
          "content-type": "application/json",
          "x-operator-action": action
        },
        method: "POST"
      });

      if (!response.ok) {
        throw new Error("Could not start the run.");
      }
      const initialStatus: unknown = await response.json();
      if (!isRunStatus(initialStatus) || !initialStatus.sessionId) {
        throw new Error("The run returned an invalid session.");
      }
      setStatus(initialStatus);
      await pollRun(initialStatus.sessionId);
    } catch {
      setStatus((current) => ({ ...current, state: "error" }));
    }
  }

  async function pollRun(sessionId: string) {
    let cursor = 0;
    while (true) {
      await new Promise((resolve) => setTimeout(resolve, POLL_INTERVAL_MS));
      const response = await fetch(
        `/eve/v1/operator/runs/${encodeURIComponent(sessionId)}/status?cursor=${cursor}`,
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
      className={`status status-${status.state}`}
      role="status"
      aria-live="polite"
    >
      <span className="statusDot" aria-hidden="true" />
      <div>
        <strong>{status.state}</strong>
        {status.sessionId ? <code>{status.sessionId}</code> : null}
        {status.models ? (
          <>
            <code>author {status.models.authorModel}</code>
            <code>reviewer {status.models.reviewerModel}</code>
          </>
        ) : null}
      </div>
    </div>
  );
}
