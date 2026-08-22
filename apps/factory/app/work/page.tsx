import { listExamples } from "../../agent/lib/examples";
import { OperatorChat } from "../operator-chat";
import { RunMaintenance } from "../run-maintenance";
import { RunPerformance } from "../run-performance";

const AGENT_RUNS_URL =
  "https://vercel.com/vercel-internal-apps/turborepo-factory/observability/agent-runs";

export default function WorkPage() {
  const examples = listExamples();
  const harnessEnabled = Boolean(process.env.GITHUB_TOKEN_EXCHANGE_URL);
  return (
    <main id="main-content" className="pageContent">
      <h1 className="pageTitle">Start work</h1>
      <section className="manualSchedules" aria-labelledby="manual-schedules-title">
        <header>
          <h2 id="manual-schedules-title">Run scheduled jobs</h2>
          <p>Start either daily job now without waiting for its next cron run.</p>
        </header>
        <div className="manualScheduleGrid">
          <section aria-labelledby="maintenance-title">
            <h3 id="maintenance-title">Daily example maintenance</h3>
            <RunMaintenance
              agentRunsUrl={AGENT_RUNS_URL}
              examples={examples}
              harnessEnabled={harnessEnabled}
            />
          </section>
          <section aria-labelledby="performance-title">
            <h3 id="performance-title">Daily performance improvement</h3>
            <RunPerformance agentRunsUrl={AGENT_RUNS_URL} />
          </section>
        </div>
      </section>
      <section className="operation" aria-labelledby="chat-title">
        <div className="operationHeader">
          <div>
            <h2 id="chat-title">Chat</h2>
          </div>
          <span className="schedule">ON DEMAND</span>
        </div>
        <p className="description">
          Opens an ad-hoc session on the factory image, with the same sandbox
          checkout of <code>main</code> the scheduled operations use. Ask for
          anything in the repository; the agent opens a draft pull request only
          when you ask for one and approve the call.
        </p>
        <dl className="facts">
          <div>
            <dt>Scope</dt>
            <dd>
              <code>whole checkout</code>
            </dd>
          </div>
          <div>
            <dt>Execution</dt>
            <dd>Chat mode</dd>
          </div>
          <div>
            <dt>Output</dt>
            <dd>Draft PR on request</dd>
          </div>
        </dl>
        <div className="controls">
          <OperatorChat agentRunsUrl={AGENT_RUNS_URL} />
        </div>
      </section>
    </main>
  );
}
