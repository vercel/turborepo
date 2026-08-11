import { RunMaintenance } from "./run-maintenance";

const AGENT_RUNS_URL =
  "https://vercel.com/vercel-internal-apps/turborepo-eve-agent/observability/agent-runs";

export default function OperatorPage() {
  return (
    <main>
      <header className="hero">
        <p className="eyebrow">Examples agent / operator</p>
        <h1>
          Maintenance
          <span>control plane</span>
        </h1>
        <p className="intro">
          Keep Turborepo examples current through one focused, rotating
          maintenance run each day.
        </p>
      </header>

      <section className="operation" aria-labelledby="operation-title">
        <div className="operationHeader">
          <div>
            <p className="eyebrow">Scheduled operation</p>
            <h2 id="operation-title">Daily example maintenance</h2>
          </div>
          <span className="schedule">DAILY · 14:00 UTC</span>
        </div>

        <p className="description">
          Selects one example by UTC date, audits and upgrades only that
          example, validates it, and opens a focused draft pull request when
          changes exist.
        </p>

        <dl className="facts">
          <div>
            <dt>Scope</dt>
            <dd>
              <code>one example/day</code>
            </dd>
          </div>
          <div>
            <dt>Execution</dt>
            <dd>Task mode</dd>
          </div>
          <div>
            <dt>Output</dt>
            <dd>Draft PR or no changes</dd>
          </div>
        </dl>

        <RunMaintenance agentRunsUrl={AGENT_RUNS_URL} />
      </section>

      <footer>
        Detailed output and diagnostics remain in Vercel Agent Runs.
      </footer>
    </main>
  );
}
