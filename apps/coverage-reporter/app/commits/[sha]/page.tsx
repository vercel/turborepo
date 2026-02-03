import Link from "next/link";
import { notFound } from "next/navigation";
import { getCoverageReport, getBranchSummary } from "@/lib/blob";
import { calculateCoverageDiff } from "@/lib/coverage";
import { StatCard } from "@/components/stat-card";
import { ProgressBar } from "@/components/progress-bar";

export const dynamic = "force-dynamic";

interface PageProps {
  params: Promise<{ sha: string }>;
}

export default async function CommitPage({ params }: PageProps) {
  const { sha } = await params;
  const report = await getCoverageReport(sha);

  if (!report) {
    notFound();
  }

  // Get baseline for diff
  const mainSummary = await getBranchSummary("main");
  const masterSummary = await getBranchSummary("master");
  const baseline = mainSummary || masterSummary;
  const diff = baseline
    ? calculateCoverageDiff(report.summary, baseline)
    : null;

  return (
    <main className="container">
      <div className="flex items-center gap-4 mb-2">
        <Link href="/" className="text-muted">
          &larr; Back
        </Link>
        <h1 style={{ margin: 0 }}>
          Commit <span className="mono">{sha.slice(0, 7)}</span>
        </h1>
      </div>
      <p className="text-muted mb-2">
        Branch: {report.branch} | {new Date(report.timestamp).toLocaleString()}
      </p>

      {/* Summary */}
      <div className="card">
        <h2>Coverage Summary</h2>
        <div className="grid grid-4">
          <StatCard
            label="Line Coverage"
            value={report.summary.lines.percent}
            delta={diff?.lines}
          />
          <StatCard
            label="Function Coverage"
            value={report.summary.functions.percent}
            delta={diff?.functions}
          />
          <StatCard
            label="Branch Coverage"
            value={report.summary.branches.percent}
            delta={diff?.branches}
          />
          <StatCard
            label="Region Coverage"
            value={report.summary.regions.percent}
          />
        </div>
        {diff && (
          <p className="text-sm text-muted" style={{ marginTop: "1rem" }}>
            Compared to main branch baseline
          </p>
        )}
      </div>

      {/* Crates Table */}
      <div className="card">
        <h2>Coverage by Crate ({report.crates.length} crates)</h2>
        <table>
          <thead>
            <tr>
              <th>Crate</th>
              <th>Lines</th>
              <th>Functions</th>
              <th>Branches</th>
              <th>Files</th>
            </tr>
          </thead>
          <tbody>
            {report.crates
              .sort((a, b) => b.summary.lines.percent - a.summary.lines.percent)
              .map((crate) => (
                <tr key={crate.name}>
                  <td className="mono">{crate.name}</td>
                  <td>
                    <div className="flex items-center gap-2">
                      <span style={{ width: "3.5rem" }}>
                        {crate.summary.lines.percent.toFixed(1)}%
                      </span>
                      <div style={{ width: "80px" }}>
                        <ProgressBar
                          percent={crate.summary.lines.percent}
                          size="sm"
                        />
                      </div>
                    </div>
                  </td>
                  <td>{crate.summary.functions.percent.toFixed(1)}%</td>
                  <td>{crate.summary.branches.percent.toFixed(1)}%</td>
                  <td className="text-muted">{crate.files.length}</td>
                </tr>
              ))}
          </tbody>
        </table>
      </div>

      {/* Files with low coverage */}
      <div className="card">
        <h2>Files with Lowest Coverage</h2>
        <table>
          <thead>
            <tr>
              <th>File</th>
              <th>Crate</th>
              <th>Lines</th>
              <th>Uncovered</th>
            </tr>
          </thead>
          <tbody>
            {report.files
              .filter((f) => f.summary.lines.total > 0)
              .sort((a, b) => a.summary.lines.percent - b.summary.lines.percent)
              .slice(0, 20)
              .map((file) => (
                <tr key={file.path}>
                  <td className="mono text-sm">{file.path}</td>
                  <td className="mono text-sm text-muted">{file.crate}</td>
                  <td>
                    <div className="flex items-center gap-2">
                      <span style={{ width: "3.5rem" }}>
                        {file.summary.lines.percent.toFixed(1)}%
                      </span>
                      <div style={{ width: "60px" }}>
                        <ProgressBar
                          percent={file.summary.lines.percent}
                          size="sm"
                        />
                      </div>
                    </div>
                  </td>
                  <td className="text-muted text-sm">
                    {file.uncoveredLines.length > 0 ? (
                      <span>
                        {file.uncoveredLines.length} lines (
                        {file.uncoveredLines.slice(0, 5).join(", ")}
                        {file.uncoveredLines.length > 5 && "..."})
                      </span>
                    ) : (
                      "-"
                    )}
                  </td>
                </tr>
              ))}
          </tbody>
        </table>
      </div>
    </main>
  );
}
