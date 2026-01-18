/**
 * Vercel Blob storage layer for coverage data
 */

import { put, list, head, del } from "@vercel/blob";
import type {
  CoverageReport,
  CoverageIndex,
  CommitEntry,
  CoverageSummary
} from "./types";

const BLOB_PREFIX = "coverage";
const INDEX_PATH = `${BLOB_PREFIX}/index.json`;
const MAX_HISTORY_ENTRIES = 100;
const MAX_BRANCH_HISTORY = 50;

/**
 * Get the blob path for a commit's coverage data
 */
function getCommitPath(sha: string): string {
  return `${BLOB_PREFIX}/commits/${sha}.json`;
}

/**
 * Get the blob path for a branch's coverage history entry
 */
function getBranchPath(branch: string, timestamp: string): string {
  // Sanitize branch name for path
  const safeBranch = branch.replace(/[^a-zA-Z0-9-_]/g, "_");
  return `${BLOB_PREFIX}/branches/${safeBranch}/${timestamp}.json`;
}

/**
 * Fetch the coverage index, or create an empty one if it doesn't exist
 */
export async function getIndex(): Promise<CoverageIndex> {
  try {
    const response = await fetch(`${process.env.BLOB_URL}/${INDEX_PATH}`);
    if (response.ok) {
      return await response.json();
    }
  } catch {
    // Index doesn't exist yet
  }

  return {
    commits: [],
    branches: {},
    updatedAt: new Date().toISOString()
  };
}

/**
 * Save the coverage index
 */
async function saveIndex(index: CoverageIndex): Promise<void> {
  index.updatedAt = new Date().toISOString();
  await put(INDEX_PATH, JSON.stringify(index, null, 2), {
    access: "public",
    addRandomSuffix: false
  });
}

/**
 * Store a coverage report
 */
export async function storeCoverageReport(
  report: CoverageReport
): Promise<{ url: string; commitUrl: string; branchUrl: string }> {
  // Store the full report at commit path
  const commitPath = getCommitPath(report.sha);
  const { url: commitUrl } = await put(
    commitPath,
    JSON.stringify(report, null, 2),
    {
      access: "public",
      addRandomSuffix: false
    }
  );

  // Store a copy at branch path for history
  const branchPath = getBranchPath(report.branch, report.timestamp);
  const { url: branchUrl } = await put(
    branchPath,
    JSON.stringify(report, null, 2),
    {
      access: "public",
      addRandomSuffix: false
    }
  );

  // Update the index
  const index = await getIndex();

  // Add/update commit entry
  const commitEntry: CommitEntry = {
    sha: report.sha,
    branch: report.branch,
    timestamp: report.timestamp,
    summary: report.summary
  };

  // Remove existing entry for this SHA if present
  index.commits = index.commits.filter((c) => c.sha !== report.sha);
  index.commits.unshift(commitEntry);

  // Trim to max entries
  if (index.commits.length > MAX_HISTORY_ENTRIES) {
    index.commits = index.commits.slice(0, MAX_HISTORY_ENTRIES);
  }

  // Update branch entry
  const safeBranch = report.branch;
  if (!index.branches[safeBranch]) {
    index.branches[safeBranch] = {
      name: safeBranch,
      latestSha: report.sha,
      latestTimestamp: report.timestamp,
      history: []
    };
  }

  const branchEntry = index.branches[safeBranch];
  branchEntry.latestSha = report.sha;
  branchEntry.latestTimestamp = report.timestamp;
  branchEntry.history.unshift(report.timestamp);

  // Trim branch history
  if (branchEntry.history.length > MAX_BRANCH_HISTORY) {
    branchEntry.history = branchEntry.history.slice(0, MAX_BRANCH_HISTORY);
  }

  await saveIndex(index);

  return {
    url: commitUrl,
    commitUrl,
    branchUrl
  };
}

/**
 * Get coverage report for a specific commit
 */
export async function getCoverageReport(
  sha: string
): Promise<CoverageReport | null> {
  try {
    const path = getCommitPath(sha);
    const response = await fetch(`${process.env.BLOB_URL}/${path}`);
    if (response.ok) {
      return await response.json();
    }
  } catch {
    // Report doesn't exist
  }
  return null;
}

/**
 * Get coverage history for a branch
 */
export async function getBranchHistory(
  branch: string,
  limit = 20
): Promise<CoverageReport[]> {
  const index = await getIndex();
  const branchEntry = index.branches[branch];

  if (!branchEntry) {
    return [];
  }

  const reports: CoverageReport[] = [];
  const timestamps = branchEntry.history.slice(0, limit);

  for (const timestamp of timestamps) {
    try {
      const path = getBranchPath(branch, timestamp);
      const response = await fetch(`${process.env.BLOB_URL}/${path}`);
      if (response.ok) {
        reports.push(await response.json());
      }
    } catch {
      // Skip failed fetches
    }
  }

  return reports;
}

/**
 * Get recent commits across all branches
 */
export async function getRecentCommits(limit = 20): Promise<CommitEntry[]> {
  const index = await getIndex();
  return index.commits.slice(0, limit);
}

/**
 * Get coverage summary for a branch (latest)
 */
export async function getBranchSummary(
  branch: string
): Promise<CoverageSummary | null> {
  const index = await getIndex();
  const branchEntry = index.branches[branch];

  if (!branchEntry) {
    return null;
  }

  const commit = index.commits.find((c) => c.sha === branchEntry.latestSha);
  return commit?.summary ?? null;
}

/**
 * List all branches with coverage data
 */
export async function listBranches(): Promise<string[]> {
  const index = await getIndex();
  return Object.keys(index.branches).sort();
}
