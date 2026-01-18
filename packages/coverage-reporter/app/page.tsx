import Link from "next/link";
import { getIndex, getBranchHistory } from "@/lib/blob";
import { CoverageChart } from "@/components/coverage-chart";
import { StatCard } from "@/components/stat-card";
import { ProgressBar } from "@/components/progress-bar";

export const dynamic = "force-dynamic";
export const revalidate = 0;

export default async function Dashboard() {
  const index = await getIndex();
  const mainHistory = await getBranchHistory("main", 30);

  // Get latest from main or master
  const latestMain = index.branches["main"] || index.branches["master"];
  const latestCommit = latestMain
    ? index.commits.find((c) => c.sha === latestMain.latestSha)
    : index.commits[0];

  // Prepare chart data (reverse for chronological order)
  const chartData = mainHistory
    .map((r) => ({
      timestamp: r.timestamp,
      lines: r.summary.lines.percent,
      functions: r.summary.functions.percent,
      branches: r.summary.branches.percent
    }))
    .reverse();

  // Calculate delta from previous
  const previousCommit = mainHistory[1];
  const lineDelta =
    previousCommit && latestCommit
      ? latestCommit.summary.lines.percent -
        previousCommit.summary.lines.percent
      : undefined;
  const funcDelta =
    previousCommit && latestCommit
      ? latestCommit.summary.functions.percent -
        previousCommit.summary.functions.percent
      : undefined;
  const branchDelta =
    previousCommit && latestCommit
      ? latestCommit.summary.branches.percent -
        previousCommit.summary.branches.percent
      : undefined;

  return (
    <main className="container">
      <h1>Coverage Dashboard</h1>

      {latestCommit ? (
        <>
          {/* Summary Stats */}
          <div className="card">
            <h2>Current Coverage (main)</h2>
            <div className="grid grid-4">
              <StatCard
                label="Line Coverage"
                value={latestCommit.summary.lines.percent}
                delta={lineDelta}
              />
              <StatCard
                label="Function Coverage"
                value={latestCommit.summary.functions.percent}
                delta={funcDelta}
              />
              <StatCard
                label="Branch Coverage"
                value={latestCommit.summary.branches.percent}
                delta={branchDelta}
              />
              <StatCard
                label="Region Coverage"
                value={latestCommit.summary.regions.percent}
              />
            </div>
            <div className="text-sm text-muted" style={{ marginTop: "1rem" }}>
              Last updated: {new Date(latestCommit.timestamp).toLocaleString()}{" "}
              (<span className="mono">{latestCommit.sha.slice(0, 7)}</span>)
            </div>
          </div>

          {/* Trend Chart */}
          {chartData.length > 1 && (
            <div className="card">
              <h2>Coverage Trend</h2>
              <CoverageChart data={chartData} />
            </div>
          )}

          {/* Recent Commits */}
          <div className="card">
            <h2>Recent Commits</h2>
            <table>
              <thead>
                <tr>
                  <th>Commit</th>
                  <th>Branch</th>
                  <th>Lines</th>
                  <th>Functions</th>
                  <th>Branches</th>
                  <th>Date</th>
                </tr>
              </thead>
              <tbody>
                {index.commits.slice(0, 10).map((commit) => (
                  <tr key={commit.sha}>
                    <td>
                      <Link href={`/commits/${commit.sha}`} className="mono">
                        {commit.sha.slice(0, 7)}
                      </Link>
                    </td>
                    <td>{commit.branch}</td>
                    <td>
                      <div className="flex items-center gap-2">
                        <span style={{ width: "3rem" }}>
                          {commit.summary.lines.percent.toFixed(1)}%
                        </span>
                        <div style={{ width: "60px" }}>
                          <ProgressBar
                            percent={commit.summary.lines.percent}
                            size="sm"
                          />
                        </div>
                      </div>
                    </td>
                    <td>{commit.summary.functions.percent.toFixed(1)}%</td>
                    <td>{commit.summary.branches.percent.toFixed(1)}%</td>
                    <td className="text-muted text-sm">
                      {new Date(commit.timestamp).toLocaleDateString()}
                    </td>
                  </tr>
                ))}
              </tbody>
            </table>
          </div>
        </>
      ) : (
        <div className="card">
          <p className="text-muted">
            No coverage data yet. Upload your first coverage report using the
            API.
          </p>
          <pre
            className="mono"
            style={{
              background: "#0a0a0a",
              padding: "1rem",
              borderRadius: "4px",
              marginTop: "1rem",
              overflow: "auto"
            }}
          >
            {`curl -X POST "$COVERAGE_API_URL/api/upload" \\
  -H "Authorization: Bearer $COVERAGE_API_TOKEN" \\
  -H "Content-Type: application/json" \\
  -d '{"sha": "abc123", "branch": "main", "report": <llvm-cov-json>}'`}
          </pre>
        </div>
      )}
    </main>
  );
}
