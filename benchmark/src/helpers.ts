import cp from "child_process";
import fs from "fs";
import path from "path";

export const REPO_ROOT = "large-monorepo";
export const REPO_ORIGIN = "https://github.com/gsoltis/large-monorepo.git";
export const REPO_PATH = path.join(process.cwd(), REPO_ROOT);
export const DEFAULT_EXEC_OPTS = { stdio: "ignore" as const, cwd: REPO_PATH };

export function setup(): void {
  // Clone repo if it doesn't exist, run clean
  if (fs.existsSync(REPO_ROOT)) {
    // reset the repo, remove all changed or untracked files
    cp.execSync(
      `cd ${REPO_ROOT} && git reset --hard HEAD && git clean -f -d -X`,
      {
        stdio: "inherit",
      }
    );
  } else {
    cp.execSync(`git clone ${REPO_ORIGIN}`, { stdio: "ignore" });
  }

  // Run install so we aren't benchmarking node_modules
  cp.execSync("yarn install", DEFAULT_EXEC_OPTS);
}

export function getCommitTimestamp(sha: string | undefined) {
  if (sha === undefined) {
    return new Date();
  }

  const buf = cp.execSync(`git show -s --format=%ci ${sha}`);
  const dateString = String(buf).trim();
  return new Date(dateString);
}

interface CommitDetails {
  commitSha: string;
  commitTimestamp: Date;
}

export function getCommitDetails(): CommitDetails {
  const sha = process.env["GITHUB_SHA"];

  return {
    commitSha: sha ?? "unknown sha",
    commitTimestamp: getCommitTimestamp(sha),
  };
}
