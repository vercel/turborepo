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
    <div className="mt-4">
      <div className="flex flex-wrap items-center gap-2.5 max-[520px]:[&>*]:w-full">
        <Button disabled={isBusy} onClick={() => void start()} type="button">
          {runLabel(status, "Run performance improvement now")}
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
    </div>
  );
}
