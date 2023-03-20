import { execSync } from "child_process";

export const skipAllCommits = [
  `[skip ci]`,
  `[ci skip]`,
  `[no ci]`,
  `[skip vercel]`,
  `[vercel skip]`,
];

export const forceAllCommits = [`[vercel deploy]`, `[vercel build]`];

export function skipWorkspaceCommits({ workspace }: { workspace: string }) {
  return [`[vercel skip ${workspace}]`];
}

export function forceWorkspaceCommits({ workspace }: { workspace: string }) {
  return [`[vercel deploy ${workspace}]`, `[vercel build ${workspace}]`];
}

export function getCommitDetails() {
  // if we're on Vercel, use the provided commit message
  if (process.env.VERCEL === "1") {
    if (process.env.VERCEL_GIT_COMMIT_MESSAGE) {
      return process.env.VERCEL_GIT_COMMIT_MESSAGE;
    }
  }
  return execSync("git show -s --format=%B").toString();
}

export function checkCommit({ workspace }: { workspace: string }): {
  result: "skip" | "deploy" | "continue" | "conflict";
  scope: "global" | "workspace";
  reason: string;
} {
  const commitMessage = getCommitDetails();
  const findInCommit = (commit: string) => commitMessage.includes(commit);

  // check workspace specific messages first
  const forceWorkspaceDeploy = forceWorkspaceCommits({ workspace }).find(
    findInCommit
  );
  const forceWorkspaceSkip = skipWorkspaceCommits({ workspace }).find(
    findInCommit
  );

  if (forceWorkspaceDeploy && forceWorkspaceSkip) {
    return {
      result: "conflict",
      scope: "workspace",
      reason: `Conflicting commit messages found: ${forceWorkspaceDeploy} and ${forceWorkspaceSkip}`,
    };
  }

  if (forceWorkspaceDeploy) {
    return {
      result: "deploy",
      scope: "workspace",
      reason: `Found commit message: ${forceWorkspaceDeploy}`,
    };
  }

  if (forceWorkspaceSkip) {
    return {
      result: "skip",
      scope: "workspace",
      reason: `Found commit message: ${forceWorkspaceSkip}`,
    };
  }

  // check global messages last
  const forceDeploy = forceAllCommits.find(findInCommit);
  const forceSkip = skipAllCommits.find(findInCommit);

  if (forceDeploy && forceSkip) {
    return {
      result: "conflict",
      scope: "global",
      reason: `Conflicting commit messages found: ${forceDeploy} and ${forceSkip}`,
    };
  }

  if (forceDeploy) {
    return {
      result: "deploy",
      scope: "global",
      reason: `Found commit message: ${forceDeploy}`,
    };
  }

  if (forceSkip) {
    return {
      result: "skip",
      scope: "global",
      reason: `Found commit message: ${forceSkip}`,
    };
  }

  return {
    result: "continue",
    scope: "global",
    reason: `No deploy or skip string found in commit message.`,
  };
}
