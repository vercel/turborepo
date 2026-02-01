/**
 * Vercel Blob storage layer for coverage data
 *
 * Uses blob list() API instead of an index file to avoid race conditions.
 * Storage structure:
 *   coverage/commits/{sha}.json - Full report per commit
 *   coverage/branches/{branch}/{timestamp}.json - Per-branch history
 */

import { put, list } from "@vercel/blob";
import type { CoverageReport, CommitEntry, CoverageSummary } from "./types";

const BLOB_PREFIX = "coverage";

/**
 * Get the blob path for a commit's coverage data
 */
function getCommitPath(sha: string): string {
  return `${BLOB_PREFIX}/commits/${sha}.json`;
}

/**
 * Sanitize branch name for use in paths
 */
function sanitizeBranch(branch: string): string {
  return branch.replace(/[^a-zA-Z0-9-_]/g, "_");
}

/**
 * Get the blob path for a branch's coverage history entry
 */
function getBranchPath(branch: string, timestamp: string): string {
  const safeBranch = sanitizeBranch(branch);
  return `${BLOB_PREFIX}/branches/${safeBranch}/${timestamp}.json`;
}

/**
 * Fetch a blob's JSON content by URL
 */
async function fetchBlob<T>(url: string): Promise<T | null> {
  try {
    const response = await fetch(url);
    if (response.ok) {
      return await response.json();
    }
  } catch {
    // Blob doesn't exist or fetch failed
  }
  return null;
}

/**
 * Store a coverage report
 */
export async function storeCoverageReport(
  report: CoverageReport
): Promise<{ url: string }> {
  const content = JSON.stringify(report, null, 2);

  // Store at commit path
  const commitPath = getCommitPath(report.sha);
  const { url } = await put(commitPath, content, {
    access: "public",
    addRandomSuffix: false
  });

  // Also store at branch path for history
  const branchPath = getBranchPath(report.branch, report.timestamp);
  await put(branchPath, content, {
    access: "public",
    addRandomSuffix: false
  });

  return { url };
}

/**
 * Get coverage report for a specific commit
 */
export async function getCoverageReport(
  sha: string
): Promise<CoverageReport | null> {
  const { blobs } = await list({ prefix: `${BLOB_PREFIX}/commits/${sha}` });
  if (blobs.length === 0) return null;
  return fetchBlob<CoverageReport>(blobs[0].url);
}

/**
 * Get recent commits across all branches
 */
export async function getRecentCommits(limit = 20): Promise<CommitEntry[]> {
  const { blobs } = await list({
    prefix: `${BLOB_PREFIX}/commits/`,
    limit: limit * 2 // Fetch extra in case some fail
  });

  // Sort by uploadedAt descending (most recent first)
  const sorted = blobs.sort(
    (a, b) =>
      new Date(b.uploadedAt).getTime() - new Date(a.uploadedAt).getTime()
  );

  const entries: CommitEntry[] = [];

  for (const blob of sorted.slice(0, limit)) {
    const report = await fetchBlob<CoverageReport>(blob.url);
    if (report) {
      entries.push({
        sha: report.sha,
        branch: report.branch,
        timestamp: report.timestamp,
        summary: report.summary
      });
    }
  }

  return entries;
}

/**
 * Get coverage history for a branch
 */
export async function getBranchHistory(
  branch: string,
  limit = 20
): Promise<CoverageReport[]> {
  const safeBranch = sanitizeBranch(branch);
  const { blobs } = await list({
    prefix: `${BLOB_PREFIX}/branches/${safeBranch}/`,
    limit: limit * 2
  });

  // Sort by uploadedAt descending
  const sorted = blobs.sort(
    (a, b) =>
      new Date(b.uploadedAt).getTime() - new Date(a.uploadedAt).getTime()
  );

  const reports: CoverageReport[] = [];

  for (const blob of sorted.slice(0, limit)) {
    const report = await fetchBlob<CoverageReport>(blob.url);
    if (report) {
      reports.push(report);
    }
  }

  return reports;
}

/**
 * Get coverage summary for a branch (latest)
 */
export async function getBranchSummary(
  branch: string
): Promise<CoverageSummary | null> {
  const history = await getBranchHistory(branch, 1);
  return history[0]?.summary ?? null;
}

/**
 * List all branches with coverage data
 */
export async function listBranches(): Promise<string[]> {
  const { blobs } = await list({
    prefix: `${BLOB_PREFIX}/branches/`
  });

  // Extract unique branch names from paths
  const branches = new Set<string>();
  for (const blob of blobs) {
    // Path format: coverage/branches/{branch}/{timestamp}.json
    const match = blob.pathname.match(/^coverage\/branches\/([^/]+)\//);
    if (match) {
      branches.add(match[1]);
    }
  }

  return Array.from(branches).sort();
}

// Legacy exports for compatibility - can remove after migration
export async function getIndex() {
  const commits = await getRecentCommits(100);
  const branchNames = await listBranches();

  const branches: Record<
    string,
    {
      name: string;
      latestSha: string;
      latestTimestamp: string;
      history: string[];
    }
  > = {};

  for (const branch of branchNames) {
    const history = await getBranchHistory(branch, 50);
    if (history.length > 0) {
      branches[branch] = {
        name: branch,
        latestSha: history[0].sha,
        latestTimestamp: history[0].timestamp,
        history: history.map((h) => h.timestamp)
      };
    }
  }

  return {
    commits,
    branches,
    updatedAt: new Date().toISOString()
  };
}
