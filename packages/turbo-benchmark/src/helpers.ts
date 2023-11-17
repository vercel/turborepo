import cp from "node:child_process";
import fs from "node:fs";
import path from "node:path";

export const REPO_ROOT = "large-monorepo";
export const REPO_ORIGIN = "https://github.com/gsoltis/large-monorepo.git";
export const REPO_PATH = path.join(process.cwd(), REPO_ROOT);
export const DEFAULT_EXEC_OPTS = { stdio: "ignore" as const, cwd: REPO_PATH };

const isWin = process.platform === "win32";

export const TURBO_BIN = path.resolve(
  path.join("..", "..", "target", "release", `turbo${isWin ? ".exe" : ""}`)
);

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
  console.log("running yarn install");
  cp.execSync("yarn install", DEFAULT_EXEC_OPTS);
}

export function getCommitDetails(): {
  commitSha: string;
  commitTimestamp: Date;
} {
  const envSha = process.env.GITHUB_SHA;
  if (envSha === undefined) {
    return {
      commitSha: "unknown sha",
      commitTimestamp: new Date(),
    };
  }
  const buf = cp.execSync(`git show -s --format=%ci ${envSha}`);
  const dateString = String(buf).trim();
  const commitTimestamp = new Date(dateString);
  return {
    commitSha: envSha,
    commitTimestamp,
  };
}

export interface TTFTData {
  name: string;
  scm: string;
  platform: string;
  cpus: number;
  startTimeUnixMicroseconds: number;
  turboVersion: string;
  durationMicroseconds: number;
  commitSha?: string;
  commitTimestamp?: Date;
  url?: string;
}

export function getTTFTData(filePath: string, runID: string): TTFTData {
  const contents = fs.readFileSync(filePath);
  const data = JSON.parse(contents.toString()) as TTFTData;

  const commitDetails = getCommitDetails();
  data.commitSha = commitDetails.commitSha;
  data.commitTimestamp = commitDetails.commitTimestamp;
  data.url = `https://github.com/vercel/turbo/actions/runs/${runID}`;
  return data;
}
