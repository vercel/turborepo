import { getGitHubToken } from "./github";
import { mutateWorkspace } from "./workspace-store";
import {
  githubPullRequestState,
  reconcileWorkspacePullRequest,
  shouldRefreshWorkspacePullRequest,
  type WorkspaceRecord
} from "./workspace";
type PullRequestLoader = (pullRequestNumber: number) => Promise<unknown>;

export async function reconcileWorkspacePullRequests(
  workspaces: readonly WorkspaceRecord[],
  now = new Date(),
  loadPullRequest: PullRequestLoader = loadGitHubPullRequest
): Promise<WorkspaceRecord[]> {
  return Promise.all(
    workspaces.map(async (workspace) => {
      if (!shouldRefreshWorkspacePullRequest(workspace, now)) return workspace;
      const pullRequestNumber = workspace.pullRequest?.number;
      if (!pullRequestNumber) return workspace;

      try {
        const state = githubPullRequestState(
          await loadPullRequest(pullRequestNumber)
        );
        if (!state) return workspace;
        const checkedAt = now.toISOString();
        return await mutateWorkspace(workspace.id, (current) =>
          reconcileWorkspacePullRequest(
            current,
            pullRequestNumber,
            state,
            checkedAt
          )
        );
      } catch (error) {
        console.error(
          `Could not refresh Factory pull request #${pullRequestNumber}.`,
          error
        );
        return workspace;
      }
    })
  );
}

async function loadGitHubPullRequest(
  pullRequestNumber: number
): Promise<unknown> {
  const response = await fetch(
    `https://api.github.com/repos/vercel/turborepo/pulls/${pullRequestNumber}`,
    {
      headers: {
        accept: "application/vnd.github+json",
        authorization: `Bearer ${await getGitHubToken()}`,
        "x-github-api-version": "2022-11-28"
      }
    }
  );
  if (!response.ok)
    throw new Error(`GitHub returned ${response.status} for the pull request.`);
  return response.json();
}
