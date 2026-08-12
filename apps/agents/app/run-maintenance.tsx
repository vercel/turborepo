"use client";

import { useState } from "react";

type RunState = "starting" | "running" | "done" | "error";
type SlackTestState =
  | { readonly state: "idle" }
  | { readonly state: "sending" }
  | {
      readonly state: "done";
      readonly channel: string;
      readonly timestamp: string | null;
    }
  | {
      readonly state: "error";
      readonly channel: string | null;
      readonly error: string;
    };

interface RunStatus {
  readonly cursor?: number;
  readonly sessionId?: string;
  readonly state: RunState;
}

interface RunMaintenanceProps {
  readonly agentRunsUrl: string;
  readonly examples: string[];
}

const MILLISECONDS_PER_DAY = 86_400_000;

function dailyExample(examples: string[]): string {
  if (examples.length === 0) return "";
  const dayNumber = Math.floor(Date.now() / MILLISECONDS_PER_DAY);
  return examples[dayNumber % examples.length] ?? "";
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

function isSlackTestResult(value: unknown): value is
  | {
      readonly ok: true;
      readonly channel: string;
      readonly timestamp: string | null;
    }
  | {
      readonly ok: false;
      readonly channel?: string | null;
      readonly error: string;
    } {
  if (typeof value !== "object" || value === null || !("ok" in value)) {
    return false;
  }

  const candidate = value as Record<string, unknown>;
  if (candidate.ok === true) {
    return (
      typeof candidate.channel === "string" &&
      (candidate.timestamp === null || typeof candidate.timestamp === "string")
    );
  }
  return (
    candidate.ok === false &&
    (candidate.channel === undefined ||
      candidate.channel === null ||
      typeof candidate.channel === "string") &&
    typeof candidate.error === "string"
  );
}

export function RunMaintenance({
  agentRunsUrl,
  examples
}: RunMaintenanceProps) {
  const [status, setStatus] = useState<RunStatus | null>(null);
  const [slackTest, setSlackTest] = useState<SlackTestState>({ state: "idle" });
  const [selectedExample, setSelectedExample] = useState(() =>
    dailyExample(examples)
  );
  const isBusy = status?.state === "starting" || status?.state === "running";

  async function testSlackDelivery() {
    setSlackTest({ state: "sending" });
    try {
      const response = await fetch("/eve/v1/operator/slack/test", {
        body: "{}",
        headers: {
          "content-type": "application/json",
          "x-operator-action": "test-slack-delivery"
        },
        method: "POST"
      });
      const result: unknown = await response
        .json()
        .catch(() => ({ ok: false, error: "Slack test request failed." }));
      if (!isSlackTestResult(result)) {
        throw new Error("Slack returned an invalid test result.");
      }
      setSlackTest(
        result.ok
          ? {
              state: "done",
              channel: result.channel,
              timestamp: result.timestamp
            }
          : {
              state: "error",
              channel: result.channel ?? null,
              error: result.error
            }
      );
    } catch (error) {
      setSlackTest({
        state: "error",
        channel: null,
        error:
          error instanceof Error
            ? error.message
            : "Could not test Slack delivery."
      });
    }
  }

  async function startRun() {
    setStatus({ state: "starting" });

    try {
      const response = await fetch("/eve/v1/operator/runs", {
        body: JSON.stringify({ example: selectedExample }),
        headers: {
          "content-type": "application/json",
          "x-operator-action": "run-daily-maintenance"
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
      <label className="examplePicker">
        <span>Example</span>
        <select
          disabled={isBusy}
          onChange={(event) => setSelectedExample(event.target.value)}
          value={selectedExample}
        >
          {examples.map((example) => (
            <option key={example} value={example}>
              {example}
              {example === dailyExample(examples) ? " — today's rotation" : ""}
            </option>
          ))}
        </select>
      </label>
      <div className="actions">
        <button
          disabled={isBusy || !selectedExample}
          onClick={() => void startRun()}
          type="button"
        >
          {status?.state === "starting"
            ? "Starting…"
            : status?.state === "running"
              ? "Running…"
              : "Run maintenance now"}
        </button>
        <button
          className="secondaryButton"
          disabled={slackTest.state === "sending"}
          onClick={() => void testSlackDelivery()}
          type="button"
        >
          {slackTest.state === "sending"
            ? "Sending Slack test…"
            : "Send Slack test"}
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
      {slackTest.state !== "idle" ? (
        <div
          aria-live={slackTest.state === "error" ? "assertive" : "polite"}
          className={`status status-${slackTest.state === "sending" ? "running" : slackTest.state}`}
          role={slackTest.state === "error" ? "alert" : "status"}
        >
          <span className="statusDot" aria-hidden="true" />
          <div>
            <strong>
              {slackTest.state === "sending"
                ? "sending Slack test"
                : slackTest.state === "done"
                  ? "Slack test delivered"
                  : "Slack test failed"}
            </strong>
            {slackTest.state === "done" ? (
              <code>
                channel {slackTest.channel}
                {slackTest.timestamp
                  ? ` · timestamp ${slackTest.timestamp}`
                  : ""}
              </code>
            ) : null}
            {slackTest.state === "error" ? (
              <code>
                {slackTest.error}
                {slackTest.channel ? ` · channel ${slackTest.channel}` : ""}
              </code>
            ) : null}
          </div>
        </div>
      ) : null}
    </div>
  );
}
