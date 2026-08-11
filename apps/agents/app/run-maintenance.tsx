"use client";

import { useState } from "react";

type RunState = "starting" | "running" | "done" | "error";

interface RunStatus {
  readonly cursor?: number;
  readonly sessionId?: string;
  readonly state: RunState;
}

interface RunMaintenanceProps {
  readonly agentRunsUrl: string;
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
    (candidate.cursor === undefined ||
      (typeof candidate.cursor === "number" &&
        Number.isInteger(candidate.cursor) &&
        candidate.cursor >= 0))
  );
}

export function RunMaintenance({ agentRunsUrl }: RunMaintenanceProps) {
  const [status, setStatus] = useState<RunStatus | null>(null);
  const isBusy = status?.state === "starting" || status?.state === "running";

  async function startRun() {
    setStatus({ state: "starting" });

    try {
      const response = await fetch("/eve/v1/operator/runs", {
        body: "{}",
        headers: {
          "content-type": "application/json",
          "x-operator-action": "run-weekly-maintenance"
        },
        method: "POST"
      });

      if (!response.ok) {
        throw new Error("Could not start the maintenance run.");
      }
      const initialStatus: unknown = await response.json();
      if (!isRunStatus(initialStatus) || !initialStatus.sessionId) {
        throw new Error("The maintenance run returned an invalid session.");
      }
      setStatus(initialStatus);
      await pollRun(initialStatus.sessionId);
    } catch {
      setStatus((current) => ({
        sessionId: current?.sessionId,
        state: "error"
      }));
    }
  }

  async function pollRun(sessionId: string) {
    let cursor = 0;
    while (true) {
      await new Promise((resolve) => setTimeout(resolve, 3000));
      const response = await fetch(
        `/eve/v1/operator/runs/${encodeURIComponent(sessionId)}/status?cursor=${cursor}`,
        { cache: "no-store" }
      );
      if (!response.ok) {
        throw new Error("Could not read the maintenance run status.");
      }
      const nextStatus: unknown = await response.json();
      if (!isRunStatus(nextStatus)) {
        throw new Error("The maintenance run returned an invalid status.");
      }
      cursor = nextStatus.cursor ?? cursor;
      setStatus(nextStatus);
      if (nextStatus.state !== "running") return;
    }
  }

  return (
    <div className="controls">
      <div className="actions">
        <button disabled={isBusy} onClick={() => void startRun()} type="button">
          {status?.state === "starting"
            ? "Starting…"
            : status?.state === "running"
              ? "Running…"
              : "Run maintenance now"}
        </button>
        <a href={agentRunsUrl} rel="noreferrer" target="_blank">
          Open Agent Runs <span aria-hidden="true">↗</span>
        </a>
      </div>

      {status ? (
        <div
          className={`status status-${status.state}`}
          role="status"
          aria-live="polite"
        >
          <span className="statusDot" aria-hidden="true" />
          <div>
            <strong>{status.state}</strong>
            {status.sessionId ? <code>{status.sessionId}</code> : null}
          </div>
        </div>
      ) : null}
    </div>
  );
}
