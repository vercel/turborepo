import Link from "next/link";
import { getIndex, getCoverageReport } from "@/lib/blob";
import { ProgressBar } from "@/components/progress-bar";

export const dynamic = "force-dynamic";

export default async function CratesPage() {
  const index = await getIndex();

  // Get latest main/master commit
  const latestMain = index.branches["main"] || index.branches["master"];
  const latestSha = latestMain?.latestSha || index.commits[0]?.sha;

  if (!latestSha) {
    return (
      <main className="container">
        <h1>Crates Coverage</h1>
        <div className="card">
          <p className="text-muted">No coverage data available.</p>
        </div>
      </main>
    );
  }

  const report = await getCoverageReport(latestSha);

  if (!report) {
    return (
      <main className="container">
        <h1>Crates Coverage</h1>
        <div className="card">
          <p className="text-muted">Could not load coverage report.</p>
        </div>
      </main>
    );
  }

  // Sort crates by line coverage
  const sortedCrates = [...report.crates].sort(
    (a, b) => b.summary.lines.percent - a.summary.lines.percent
  );

  // Group by coverage level
  const highCoverage = sortedCrates.filter(
    (c) => c.summary.lines.percent >= 80
  );
  const mediumCoverage = sortedCrates.filter(
    (c) => c.summary.lines.percent >= 50 && c.summary.lines.percent < 80
  );
  const lowCoverage = sortedCrates.filter((c) => c.summary.lines.percent < 50);

  return (
    <main className="container">
      <h1>Crates Coverage</h1>
      <p className="text-muted mb-2">
        Coverage breakdown for {report.crates.length} crates from commit{" "}
        <Link href={`/commits/${report.sha}`} className="mono">
          {report.sha.slice(0, 7)}
        </Link>
      </p>

      {/* Summary */}
      <div className="card">
        <div className="grid grid-4">
          <div className="stat">
            <div className="stat-value">{report.crates.length}</div>
            <div className="stat-label">Total Crates</div>
          </div>
          <div className="stat">
            <div className="stat-value" style={{ color: "var(--success)" }}>
              {highCoverage.length}
            </div>
            <div className="stat-label">High Coverage (&ge;80%)</div>
          </div>
          <div className="stat">
            <div className="stat-value" style={{ color: "var(--warning)" }}>
              {mediumCoverage.length}
            </div>
            <div className="stat-label">Medium (50-80%)</div>
          </div>
          <div className="stat">
            <div className="stat-value" style={{ color: "var(--error)" }}>
              {lowCoverage.length}
            </div>
            <div className="stat-label">Low (&lt;50%)</div>
          </div>
        </div>
      </div>

      {/* Crates Table */}
      <div className="card">
        <h2>All Crates</h2>
        <table>
          <thead>
            <tr>
              <th>Crate</th>
              <th style={{ width: "200px" }}>Line Coverage</th>
              <th>Lines</th>
              <th>Functions</th>
              <th>Branches</th>
              <th>Files</th>
            </tr>
          </thead>
          <tbody>
            {sortedCrates.map((crate) => {
              const badge =
                crate.summary.lines.percent >= 80
                  ? "badge-success"
                  : crate.summary.lines.percent >= 50
                    ? "badge-warning"
                    : "badge-error";

              return (
                <tr key={crate.name}>
                  <td>
                    <span className="mono">{crate.name}</span>
                  </td>
                  <td>
                    <div className="flex items-center gap-2">
                      <span
                        className={`badge ${badge}`}
                        style={{ width: "4rem", textAlign: "center" }}
                      >
                        {crate.summary.lines.percent.toFixed(1)}%
                      </span>
                      <div style={{ flex: 1 }}>
                        <ProgressBar percent={crate.summary.lines.percent} />
                      </div>
                    </div>
                  </td>
                  <td className="text-sm text-muted">
                    {crate.summary.lines.covered}/{crate.summary.lines.total}
                  </td>
                  <td className="text-sm">
                    {crate.summary.functions.percent.toFixed(1)}%
                  </td>
                  <td className="text-sm">
                    {crate.summary.branches.percent.toFixed(1)}%
                  </td>
                  <td className="text-sm text-muted">{crate.files.length}</td>
                </tr>
              );
            })}
          </tbody>
        </table>
      </div>
    </main>
  );
}
