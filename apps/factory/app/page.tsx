const AGENT_RUNS_URL =
  "https://vercel.com/vercel-internal-apps/turborepo-factory/observability/agent-runs";

const DEMO_RUNS = [
  {
    created: "Today, 15:30 UTC",
    status: "Completed",
    title: "Daily performance improvement",
    tokens: "2.1M",
    trigger: "Schedule",
    turns: "1"
  },
  {
    created: "Today, 14:00 UTC",
    status: "Completed",
    title: "Daily example maintenance",
    tokens: "1.3M",
    trigger: "Schedule",
    turns: "1"
  },
  {
    created: "Yesterday, 16:20 UTC",
    status: "Completed",
    title: "Add a Linear connector to the factory",
    tokens: "3.7M",
    trigger: "HTTP",
    turns: "6"
  }
];

export default function OperatorPage() {
  return (
    <main
      id="main-content"
      className="mx-auto w-[min(1200px,calc(100%_-_48px))] max-[720px]:w-[min(1200px,calc(100%_-_32px))]"
    >
      <section
        className="max-w-[560px] py-16"
        aria-labelledby="agent-runs-title"
      >
        <h1
          className="text-pretty text-[clamp(1.5rem,3vw,2rem)] leading-tight font-semibold tracking-[-0.04em]"
          id="agent-runs-title"
        >
          Agent Runs
        </h1>
        <p className="my-3 mb-6 text-pretty text-muted-foreground">
          View scheduled jobs, operator chats, and their full execution history
          in Vercel Observability.
        </p>
        <a
          className="inline-flex min-h-9 items-center rounded-md bg-primary px-3 text-sm font-medium text-primary-foreground no-underline"
          href={AGENT_RUNS_URL}
          rel="noreferrer"
          target="_blank"
        >
          Open Agent Runs <span className="sr-only">in a new tab</span>
        </a>
        <div className="mt-12" aria-labelledby="demo-runs-title">
          <div className="flex items-center justify-between">
            <h2
              className="text-pretty text-base font-semibold tracking-[-0.02em]"
              id="demo-runs-title"
            >
              Example Agent Runs
            </h2>
            <span className="font-mono text-[0.6875rem] text-warning">
              DEMO DATA
            </span>
          </div>
          <p className="mt-1 mb-3 text-[0.8125rem] text-pretty">
            Illustrative only. Open Agent Runs for live data.
          </p>
          <div className="overflow-x-auto rounded-md border border-border">
            <table className="w-full border-collapse text-[0.8125rem]">
              <thead>
                <tr>
                  <th className="border-b border-border px-3 py-2.5 text-left whitespace-nowrap text-[0.6875rem] font-medium text-muted-foreground uppercase">
                    Created
                  </th>
                  <th className="border-b border-border px-3 py-2.5 text-left whitespace-nowrap text-[0.6875rem] font-medium text-muted-foreground uppercase">
                    Trigger
                  </th>
                  <th className="border-b border-border px-3 py-2.5 text-left whitespace-nowrap text-[0.6875rem] font-medium text-muted-foreground uppercase">
                    Run
                  </th>
                  <th className="border-b border-border px-3 py-2.5 text-left whitespace-nowrap text-[0.6875rem] font-medium text-muted-foreground uppercase">
                    Tokens
                  </th>
                  <th className="border-b border-border px-3 py-2.5 text-left whitespace-nowrap text-[0.6875rem] font-medium text-muted-foreground uppercase">
                    Turns
                  </th>
                  <th className="border-b border-border px-3 py-2.5 text-left whitespace-nowrap text-[0.6875rem] font-medium text-muted-foreground uppercase">
                    Status
                  </th>
                </tr>
              </thead>
              <tbody>
                {DEMO_RUNS.map((run) => (
                  <tr key={run.title} className="last:[&>td]:border-b-0">
                    <td className="border-b border-border px-3 py-2.5 text-left whitespace-nowrap">
                      {run.created}
                    </td>
                    <td className="border-b border-border px-3 py-2.5 text-left whitespace-nowrap">
                      {run.trigger}
                    </td>
                    <td className="border-b border-border px-3 py-2.5 text-left whitespace-nowrap">
                      {run.title}
                    </td>
                    <td className="border-b border-border px-3 py-2.5 text-left whitespace-nowrap">
                      {run.tokens}
                    </td>
                    <td className="border-b border-border px-3 py-2.5 text-left whitespace-nowrap">
                      {run.turns}
                    </td>
                    <td className="border-b border-border px-3 py-2.5 text-left whitespace-nowrap">
                      {run.status}
                    </td>
                  </tr>
                ))}
              </tbody>
            </table>
          </div>
        </div>
      </section>
    </main>
  );
}
