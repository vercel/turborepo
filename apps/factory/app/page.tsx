import { listExamples } from "../agent/lib/examples";
import { listControlPlaneSnapshot } from "../agent/lib/run-registry";
import { RunMaintenance } from "./run-maintenance";
import { RunObservatory } from "./run-observatory";
import { RunPerformance } from "./run-performance";

const AGENT_RUNS_URL =
  "https://vercel.com/vercel-internal-apps/turborepo-eve-agent/observability/agent-runs";

export const dynamic = "force-dynamic";

export default async function OperatorPage() {
  const examples = listExamples();
  const snapshot = await listControlPlaneSnapshot();
  const harnessEnabled = Boolean(process.env.GITHUB_TOKEN_EXCHANGE_URL);
  return (
    <main id="main-content">
      <header className="masthead">
        <div className="brand" aria-label="Turborepo Factory">
          <span className="brandMark" aria-hidden="true" />
          <span>Turborepo Factory</span>
        </div>
        <p>Agent operations</p>
      </header>

      <header className="hero">
        <h1>Keep examples current and hot paths fast.</h1>
        <p className="intro">
          Observe every agent run, inspect its sandbox, and start the two daily
          operations that maintain the Turborepo repository.
        </p>
      </header>

      <RunObservatory initialSnapshot={snapshot} />

      <section className="operation" aria-labelledby="operation-title">
        <div className="operationHeader">
          <div>
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

        <RunMaintenance
          agentRunsUrl={AGENT_RUNS_URL}
          examples={examples}
          harnessEnabled={harnessEnabled}
        />
      </section>

      <section className="operation" aria-labelledby="performance-title">
        <div className="operationHeader">
          <div>
            <h2 id="performance-title">Daily performance improvement</h2>
          </div>
          <span className="schedule">DAILY · 15:30 UTC</span>
        </div>

        <p className="description">
          Finds one focused Turborepo performance win, records a baseline and an
          identical after measurement plus correctness validation, and publishes
          a draft pull request only once the opposite model approves the exact
          final diff. GPT 5.6 Sol authors on even UTC days and Claude Fable 5
          authors on odd UTC days; the other model reviews.
        </p>

        <dl className="facts">
          <div>
            <dt>Scope</dt>
            <dd>
              <code>one measured change/day</code>
            </dd>
          </div>
          <div>
            <dt>Review</dt>
            <dd>Adversarial opposite-model subagent</dd>
          </div>
          <div>
            <dt>Output</dt>
            <dd>Draft PR or no change</dd>
          </div>
        </dl>

        <RunPerformance agentRunsUrl={AGENT_RUNS_URL} />
      </section>

      <footer className="siteFooter">
        <span className="brandMark" aria-hidden="true" />
        <span>Detailed output is linked from each run.</span>
      </footer>
    </main>
  );
}
