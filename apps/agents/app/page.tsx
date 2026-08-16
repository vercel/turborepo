import { listExamples } from "../agent/lib/examples";
import { RunMaintenance } from "./run-maintenance";
import { RunPerformance } from "./run-performance";

const AGENT_RUNS_URL =
  "https://vercel.com/vercel-internal-apps/turborepo-eve-agent/observability/agent-runs";

export default function OperatorPage() {
  const examples = listExamples();
  const openCodeAuthConfigured = Boolean(process.env.OPENCODE_SERVER_TOKEN) !==
    Boolean(process.env.OPENCODE_SERVER_PASSWORD);
  const openCodeEnabled = Boolean(
    process.env.OPENCODE_SERVER_URL &&
    process.env.OPERATOR_RUN_SECRET &&
    openCodeAuthConfigured
  );
  return (
    <main>
      <header className="hero">
        <p className="eyebrow">Turborepo agent / operator</p>
        <h1>
          Automation
          <span>control plane</span>
        </h1>
        <p className="intro">
          Trigger the daily runs that keep Turborepo examples current and its
          hot paths measurably faster.
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

        <RunMaintenance
          agentRunsUrl={AGENT_RUNS_URL}
          examples={examples}
          openCodeEnabled={openCodeEnabled}
        />
      </section>

      <section className="operation" aria-labelledby="performance-title">
        <div className="operationHeader">
          <div>
            <p className="eyebrow">Scheduled operation</p>
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

      <footer>
        Detailed output and diagnostics remain in Vercel Agent Runs.
      </footer>
    </main>
  );
}
