/**
 * Parse and transform cargo-llvm-cov JSON output into our storage format
 */

import type {
  LlvmCovReport,
  LlvmCovFile,
  LlvmCovSummary,
  CoverageReport,
  CoverageSummary,
  CoverageMetric,
  CrateCoverage,
  FileCoverage
} from "./types";

/**
 * Extract crate name from a file path
 * Examples:
 *   crates/turborepo-lib/src/foo.rs -> turborepo-lib
 *   packages/turbo-repository/rust/src/bar.rs -> turbo-repository
 */
export function extractCrateName(filePath: string): string {
  // Match crates/{name}/ pattern
  const cratesMatch = filePath.match(/crates\/([^/]+)\//);
  if (cratesMatch) {
    return cratesMatch[1];
  }

  // Match packages/{name}/rust/ pattern
  const packagesMatch = filePath.match(/packages\/([^/]+)\/rust\//);
  if (packagesMatch) {
    return packagesMatch[1];
  }

  // Fallback: use first directory component
  const parts = filePath.split("/");
  return parts[0] || "unknown";
}

/**
 * Convert llvm-cov summary to our metric format
 */
function convertSummary(summary: LlvmCovSummary): CoverageSummary {
  const toMetric = (s: {
    covered: number;
    count: number;
    percent: number;
  }): CoverageMetric => ({
    covered: s.covered,
    total: s.count,
    percent: s.percent
  });

  return {
    lines: toMetric(summary.lines),
    functions: toMetric(summary.functions),
    branches: toMetric(summary.branches),
    regions: toMetric(summary.regions)
  };
}

/**
 * Extract uncovered line numbers from segments
 * Segments format: [line, col, count, hasCount, isRegionEntry, isGapRegion]
 */
function extractUncoveredLines(file: LlvmCovFile): number[] {
  const uncovered: Set<number> = new Set();
  let currentLine = 0;
  let currentCount = 0;

  for (const segment of file.segments) {
    const [line, , count, hasCount] = segment;

    // Track lines between last position and this segment
    if (currentCount === 0 && hasCount) {
      for (let l = currentLine + 1; l <= line; l++) {
        uncovered.add(l);
      }
    }

    currentLine = line;
    if (hasCount) {
      currentCount = count;
      if (count === 0) {
        uncovered.add(line);
      }
    }
  }

  return Array.from(uncovered).sort((a, b) => a - b);
}

/**
 * Parse cargo-llvm-cov JSON report into our storage format
 */
export function parseCoverageReport(
  report: LlvmCovReport,
  sha: string,
  branch: string
): CoverageReport {
  const data = report.data[0];
  if (!data) {
    throw new Error("Invalid coverage report: no data");
  }

  // Process files and group by crate
  const crateMap = new Map<
    string,
    { files: FileCoverage[]; summary: LlvmCovSummary | null }
  >();

  const files: FileCoverage[] = data.files.map((file) => {
    const crateName = extractCrateName(file.filename);
    const fileCoverage: FileCoverage = {
      path: file.filename,
      crate: crateName,
      summary: convertSummary(file.summary),
      uncoveredLines: extractUncoveredLines(file)
    };

    // Aggregate into crate
    if (!crateMap.has(crateName)) {
      crateMap.set(crateName, { files: [], summary: null });
    }
    crateMap.get(crateName)!.files.push(fileCoverage);

    return fileCoverage;
  });

  // Build crate summaries
  const crates: CrateCoverage[] = Array.from(crateMap.entries()).map(
    ([name, { files: crateFiles }]) => {
      // Aggregate metrics across files
      const aggregated = {
        lines: { covered: 0, total: 0 },
        functions: { covered: 0, total: 0 },
        branches: { covered: 0, total: 0 },
        regions: { covered: 0, total: 0 }
      };

      for (const file of crateFiles) {
        aggregated.lines.covered += file.summary.lines.covered;
        aggregated.lines.total += file.summary.lines.total;
        aggregated.functions.covered += file.summary.functions.covered;
        aggregated.functions.total += file.summary.functions.total;
        aggregated.branches.covered += file.summary.branches.covered;
        aggregated.branches.total += file.summary.branches.total;
        aggregated.regions.covered += file.summary.regions.covered;
        aggregated.regions.total += file.summary.regions.total;
      }

      const toMetric = (s: {
        covered: number;
        total: number;
      }): CoverageMetric => ({
        covered: s.covered,
        total: s.total,
        percent: s.total > 0 ? (s.covered / s.total) * 100 : 0
      });

      return {
        name,
        summary: {
          lines: toMetric(aggregated.lines),
          functions: toMetric(aggregated.functions),
          branches: toMetric(aggregated.branches),
          regions: toMetric(aggregated.regions)
        },
        files: crateFiles.map((f) => f.path)
      };
    }
  );

  // Sort crates by name
  crates.sort((a, b) => a.name.localeCompare(b.name));

  return {
    sha,
    branch,
    timestamp: new Date().toISOString(),
    summary: convertSummary(data.totals),
    crates,
    files
  };
}

/**
 * Calculate the diff between two coverage reports
 */
export function calculateCoverageDiff(
  current: CoverageSummary,
  baseline: CoverageSummary
): {
  lines: number;
  functions: number;
  branches: number;
} {
  return {
    lines: current.lines.percent - baseline.lines.percent,
    functions: current.functions.percent - baseline.functions.percent,
    branches: current.branches.percent - baseline.branches.percent
  };
}
