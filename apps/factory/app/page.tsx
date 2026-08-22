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
    <main id="main-content" className="pageContent">
      <section className="auditLogLanding" aria-labelledby="agent-runs-title">
        <h1 id="agent-runs-title">Agent Runs</h1>
        <p>
          View scheduled jobs, operator chats, and their full execution history
          in Vercel Observability.
        </p>
        <a href={AGENT_RUNS_URL} rel="noreferrer" target="_blank">
          Open Agent Runs <span className="visuallyHidden">in a new tab</span>
        </a>
        <div className="demoRuns" aria-labelledby="demo-runs-title">
          <div className="demoRunsHeader">
            <h2 id="demo-runs-title">Example Agent Runs</h2>
            <span>DEMO DATA</span>
          </div>
          <p>Illustrative only. Open Agent Runs for live data.</p>
          <div className="demoRunsViewport">
            <table>
              <thead>
                <tr>
                  <th>Created</th>
                  <th>Trigger</th>
                  <th>Run</th>
                  <th>Tokens</th>
                  <th>Turns</th>
                  <th>Status</th>
                </tr>
              </thead>
              <tbody>
                {DEMO_RUNS.map((run) => (
                  <tr key={run.title}>
                    <td>{run.created}</td>
                    <td>{run.trigger}</td>
                    <td>{run.title}</td>
                    <td>{run.tokens}</td>
                    <td>{run.turns}</td>
                    <td>{run.status}</td>
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
