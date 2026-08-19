"use client";

import { PERFORMANCE_RUN_ACTION } from "../agent/lib/operator-runs";
import { Button } from "../components/ui/button";
import { RunStatusPanel, runLabel, useOperatorRun } from "./operator-run";

interface RunPerformanceProps {
  readonly agentRunsUrl: string;
}

export function RunPerformance({ agentRunsUrl }: RunPerformanceProps) {
  const { isBusy, start, status } = useOperatorRun(PERFORMANCE_RUN_ACTION);

  return (
    <div className="controls">
      <div className="actions">
        <Button disabled={isBusy} onClick={() => void start()} type="button">
          {runLabel(status, "Run performance improvement now")}
        </Button>
        <a href={agentRunsUrl} rel="noreferrer" target="_blank">
          Open Agent Runs <span className="visuallyHidden">in a new tab</span>
          <span aria-hidden="true">↗</span>
        </a>
      </div>

      <RunStatusPanel status={status} />
    </div>
  );
}
