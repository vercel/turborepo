#!/usr/bin/env node

/**
 * Transforms LCOV coverage data into a JSON format for upload to Vercel Blob.
 *
 * Usage: node transform-coverage.js <input.lcov> <output.json>
 *
 * Environment variables:
 *   SHA       - Git commit SHA
 *   BRANCH    - Git branch name
 *   TIMESTAMP - ISO timestamp
 */

const fs = require("fs");

const [inputFile, outputFile] = process.argv.slice(2);

if (!inputFile || !outputFile) {
  console.error("Usage: node transform-coverage.js <input.lcov> <output.json>");
  process.exit(1);
}

const lcov = fs.readFileSync(inputFile, "utf8");
const files = [];
let currentFile = null;

for (const line of lcov.split("\n")) {
  if (line.startsWith("SF:")) {
    currentFile = {
      path: line.slice(3),
      lines: { covered: 0, total: 0 },
      functions: { covered: 0, total: 0 },
      branches: { covered: 0, total: 0 }
    };
  } else if (line.startsWith("LF:")) {
    currentFile.lines.total = parseInt(line.slice(3), 10);
  } else if (line.startsWith("LH:")) {
    currentFile.lines.covered = parseInt(line.slice(3), 10);
  } else if (line.startsWith("FNF:")) {
    currentFile.functions.total = parseInt(line.slice(4), 10);
  } else if (line.startsWith("FNH:")) {
    currentFile.functions.covered = parseInt(line.slice(4), 10);
  } else if (line.startsWith("BRF:")) {
    currentFile.branches.total = parseInt(line.slice(4), 10);
  } else if (line.startsWith("BRH:")) {
    currentFile.branches.covered = parseInt(line.slice(4), 10);
  } else if (line === "end_of_record" && currentFile) {
    files.push(currentFile);
    currentFile = null;
  }
}

const totals = {
  lines: { covered: 0, total: 0 },
  functions: { covered: 0, total: 0 },
  branches: { covered: 0, total: 0 }
};

for (const file of files) {
  totals.lines.covered += file.lines.covered;
  totals.lines.total += file.lines.total;
  totals.functions.covered += file.functions.covered;
  totals.functions.total += file.functions.total;
  totals.branches.covered += file.branches.covered;
  totals.branches.total += file.branches.total;
}

function toMetric(s) {
  return {
    covered: s.covered,
    total: s.total,
    percent: s.total > 0 ? (s.covered / s.total) * 100 : 0
  };
}

function getCrateName(filePath) {
  const cratesMatch = filePath.match(/crates\/([^/]+)\//);
  if (cratesMatch) return cratesMatch[1];
  const packagesMatch = filePath.match(/packages\/([^/]+)\/rust\//);
  if (packagesMatch) return packagesMatch[1];
  return "unknown";
}

const crateMap = new Map();
const fileResults = files.map((file) => {
  const crateName = getCrateName(file.path);
  const fileCoverage = {
    path: file.path,
    crate: crateName,
    summary: {
      lines: toMetric(file.lines),
      functions: toMetric(file.functions),
      branches: toMetric(file.branches),
      regions: { covered: 0, total: 0, percent: 0 }
    },
    uncoveredLines: []
  };
  if (!crateMap.has(crateName)) crateMap.set(crateName, { files: [] });
  crateMap.get(crateName).files.push(fileCoverage);
  return fileCoverage;
});

const crates = Array.from(crateMap.entries())
  .map(([name, { files: crateFiles }]) => {
    const aggregated = {
      lines: { covered: 0, total: 0 },
      functions: { covered: 0, total: 0 },
      branches: { covered: 0, total: 0 }
    };
    for (const file of crateFiles) {
      aggregated.lines.covered += file.summary.lines.covered;
      aggregated.lines.total += file.summary.lines.total;
      aggregated.functions.covered += file.summary.functions.covered;
      aggregated.functions.total += file.summary.functions.total;
      aggregated.branches.covered += file.summary.branches.covered;
      aggregated.branches.total += file.summary.branches.total;
    }
    return {
      name,
      summary: {
        lines: toMetric(aggregated.lines),
        functions: toMetric(aggregated.functions),
        branches: toMetric(aggregated.branches),
        regions: { covered: 0, total: 0, percent: 0 }
      },
      files: crateFiles.map((f) => f.path)
    };
  })
  .sort((a, b) => a.name.localeCompare(b.name));

const result = {
  sha: process.env.SHA,
  branch: process.env.BRANCH,
  timestamp: process.env.TIMESTAMP,
  summary: {
    lines: toMetric(totals.lines),
    functions: toMetric(totals.functions),
    branches: toMetric(totals.branches),
    regions: { covered: 0, total: 0, percent: 0 }
  },
  crates,
  files: fileResults
};

fs.writeFileSync(outputFile, JSON.stringify(result, null, 2));

// Output env vars for GitHub Actions
console.log(`COVERAGE_LINES=${result.summary.lines.percent}`);
console.log(`COVERAGE_FUNCTIONS=${result.summary.functions.percent}`);
console.log(`COVERAGE_BRANCHES=${result.summary.branches.percent}`);
