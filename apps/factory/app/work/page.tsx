import type { Metadata } from "next";

import { listExamples } from "../../agent/lib/examples";
import { RunMaintenance } from "../run-maintenance";
import { RunPerformance } from "../run-performance";
import { WorkspaceComposer } from "../workspace-composer";

export const metadata: Metadata = {
  title: "Start work"
};

const AGENT_RUNS_URL =
  "https://vercel.com/vercel-internal-apps/turborepo-factory/observability/agent-runs";

export default function WorkPage() {
  const examples = listExamples();
  return (
    <main
      id="main-content"
      className="mx-auto w-[min(1200px,calc(100%_-_48px))] max-[720px]:w-[min(1200px,calc(100%_-_32px))]"
    >
      <h1 className="mt-8 text-[clamp(1.5rem,3vw,2rem)] leading-tight font-semibold tracking-[-0.04em]">
        Start work
      </h1>
      <section className="py-6 pb-16" aria-labelledby="manual-schedules-title">
        <header>
          <h2
            className="text-[clamp(1.5rem,3vw,2rem)] leading-tight font-semibold tracking-[-0.04em]"
            id="manual-schedules-title"
          >
            Run scheduled jobs
          </h2>
          <p className="mt-2 mb-6 text-sm text-muted-foreground text-pretty">
            Start either daily job now without waiting for its next cron run.
          </p>
        </header>
        <div className="grid grid-cols-2 gap-6 max-[720px]:grid-cols-1">
          <section
            className="rounded-md bg-secondary p-5"
            aria-labelledby="maintenance-title"
          >
            <h3
              className="text-pretty text-base font-semibold tracking-[-0.02em]"
              id="maintenance-title"
            >
              Daily example maintenance
            </h3>
            <RunMaintenance agentRunsUrl={AGENT_RUNS_URL} examples={examples} />
          </section>
          <section
            className="rounded-md bg-secondary p-5"
            aria-labelledby="performance-title"
          >
            <h3
              className="text-pretty text-base font-semibold tracking-[-0.02em]"
              id="performance-title"
            >
              Daily performance improvement
            </h3>
            <RunPerformance agentRunsUrl={AGENT_RUNS_URL} />
          </section>
        </div>
      </section>
      <section
        className="grid grid-cols-[minmax(220px,4fr)_minmax(0,8fr)] gap-x-16 border-b border-border py-[clamp(64px,10vw,120px)] max-[720px]:block max-[720px]:py-16"
        aria-labelledby="chat-title"
      >
        <div className="contents max-[720px]:flex max-[720px]:items-start max-[720px]:justify-between max-[720px]:gap-4 max-[520px]:flex-col-reverse">
          <div className="col-start-1 row-span-4 max-[720px]:block">
            <h2
              className="text-[clamp(1.5rem,3vw,2rem)] leading-tight font-semibold tracking-[-0.04em]"
              id="chat-title"
            >
              New workspace
            </h2>
          </div>
          <span className="col-start-2 self-start justify-self-start font-mono text-xs tracking-[0.02em] text-muted-foreground">
            ON DEMAND
          </span>
        </div>
        <p className="col-start-2 mt-6 max-w-[68ch] text-muted-foreground text-pretty max-[720px]:mt-6">
          Creates a durable Factory workspace on the factory image, with the
          same sandbox checkout of <code>main</code> the scheduled operations
          use. Return later to review the transcript and continue the work in
          the same durable Eve session.
        </p>
        <dl className="col-start-2 my-10 grid grid-cols-3 gap-6 max-[520px]:grid-cols-1 max-[520px]:gap-4">
          <div className="min-w-0 border-t border-border pt-3">
            <dt className="text-xs text-muted-foreground">Scope</dt>
            <dd className="mt-1.5 text-sm">
              <code>durable sandbox</code>
            </dd>
          </div>
          <div className="min-w-0 border-t border-border pt-3">
            <dt className="text-xs text-muted-foreground">Execution</dt>
            <dd className="mt-1.5 text-sm">Multi-turn</dd>
          </div>
          <div className="min-w-0 border-t border-border pt-3">
            <dt className="text-xs text-muted-foreground">Output</dt>
            <dd className="mt-1.5 text-sm">Durable transcript</dd>
          </div>
        </dl>
        <div className="col-start-2 border-t border-border pt-8">
          <WorkspaceComposer />
        </div>
      </section>
    </main>
  );
}
