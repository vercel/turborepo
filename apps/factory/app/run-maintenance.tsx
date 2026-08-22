"use client";

import { useState } from "react";

import { HARNESS_IDS, type HarnessId } from "../agent/lib/harnesses";
import { MAINTENANCE_RUN_ACTION } from "../agent/lib/operator-runs";
import { Button } from "../components/ui/button";
import { RunStatusPanel, runLabel, useOperatorRun } from "./operator-run";
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

interface RunMaintenanceProps {
  readonly agentRunsUrl: string;
  readonly examples: string[];
  readonly harnessEnabled: boolean;
}

const MILLISECONDS_PER_DAY = 86_400_000;

function dailyExample(examples: string[]): string {
  if (examples.length === 0) return "";
  const dayNumber = Math.floor(Date.now() / MILLISECONDS_PER_DAY);
  return examples[dayNumber % examples.length] ?? "";
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
  examples,
  harnessEnabled
}: RunMaintenanceProps) {
  const { isBusy, start, status } = useOperatorRun(
    MAINTENANCE_RUN_ACTION,
    harnessEnabled ? "/api/harness/runs" : undefined
  );
  const [harness, setHarness] = useState<HarnessId>("opencode");
  const [slackTest, setSlackTest] = useState<SlackTestState>({ state: "idle" });
  const [selectedExample, setSelectedExample] = useState(() =>
    dailyExample(examples)
  );

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

  return (
    <div className="mt-4">
      <label className="mb-5 grid gap-2">
        <span className="text-[0.8125rem] font-medium text-muted-foreground">
          Example
        </span>
        <select
          className="min-h-10 w-[min(100%,440px)] rounded-md border border-input bg-background px-3 pr-9 text-foreground focus-visible:outline-2 focus-visible:outline-ring focus-visible:outline-offset-3"
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
      {harnessEnabled ? (
        <label className="mb-5 grid gap-2">
          <span className="text-[0.8125rem] font-medium text-muted-foreground">
            Harness
          </span>
          <select
            className="min-h-10 w-[min(100%,440px)] rounded-md border border-input bg-background px-3 pr-9 text-foreground focus-visible:outline-2 focus-visible:outline-ring focus-visible:outline-offset-3"
            disabled={isBusy}
            onChange={(event) => setHarness(event.target.value as HarnessId)}
            value={harness}
          >
            {HARNESS_IDS.map((id) => (
              <option key={id} value={id}>
                {id}
              </option>
            ))}
          </select>
        </label>
      ) : null}
      <div className="flex flex-wrap items-center gap-2.5 max-[520px]:[&>*]:w-full">
        <Button
          disabled={isBusy || !selectedExample}
          onClick={() =>
            void start({
              example: selectedExample,
              harness,
              sandbox: "vercel"
            })
          }
          type="button"
        >
          {runLabel(status, "Run maintenance now")}
        </Button>
        <Button
          disabled={slackTest.state === "sending"}
          onClick={() => void testSlackDelivery()}
          type="button"
          variant="outline"
        >
          {slackTest.state === "sending"
            ? "Sending Slack test…"
            : "Send Slack test"}
        </Button>
        <a
          className="inline-flex min-h-10 items-center gap-1.5 px-2 text-sm font-medium text-foreground no-underline hover:underline hover:underline-offset-4"
          href={agentRunsUrl}
          rel="noreferrer"
          target="_blank"
        >
          Open Agent Runs <span className="sr-only">in a new tab</span>
          <span aria-hidden="true">↗</span>
        </a>
      </div>

      <RunStatusPanel status={status} />
      {slackTest.state !== "idle" ? (
        <div
          aria-live={slackTest.state === "error" ? "assertive" : "polite"}
          className={`mt-6 flex items-start gap-3 rounded-md bg-muted p-3.5 text-[0.8125rem] ${slackTest.state === "sending" ? "text-warning" : slackTest.state === "done" ? "text-success" : "text-destructive"}`}
          role={slackTest.state === "error" ? "alert" : "status"}
        >
          <span
            className="mt-1.5 size-[7px] shrink-0 rounded-full bg-current"
            aria-hidden="true"
          />
          <div>
            <strong className="block font-semibold capitalize">
              {slackTest.state === "sending"
                ? "sending Slack test"
                : slackTest.state === "done"
                  ? "Slack test delivered"
                  : "Slack test failed"}
            </strong>
            {slackTest.state === "done" ? (
              <code className="mt-1 block wrap-anywhere font-mono text-xs text-muted-foreground">
                channel {slackTest.channel}
                {slackTest.timestamp
                  ? ` · timestamp ${slackTest.timestamp}`
                  : ""}
              </code>
            ) : null}
            {slackTest.state === "error" ? (
              <code className="mt-1 block wrap-anywhere font-mono text-xs text-muted-foreground">
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
