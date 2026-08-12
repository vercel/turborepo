"use client";

import { useState } from "react";

import { MAINTENANCE_RUN_ACTION } from "../agent/lib/operator-runs";
import { RunStatusPanel, runLabel, useOperatorRun } from "./operator-run";

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

export function RunMaintenance({
  agentRunsUrl,
  examples
}: RunMaintenanceProps) {
  const { isBusy, start, status } = useOperatorRun(MAINTENANCE_RUN_ACTION);
  const [selectedExample, setSelectedExample] = useState(() =>
    dailyExample(examples)
  );

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
          onClick={() => void start({ example: selectedExample })}
          type="button"
        >
          {runLabel(status, "Run maintenance now")}
        </button>
        <a href={agentRunsUrl} rel="noreferrer" target="_blank">
          Open Agent Runs <span aria-hidden="true">↗</span>
        </a>
      </div>

      <RunStatusPanel status={status} />
    </div>
  );
}
