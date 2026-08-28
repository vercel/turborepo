import { githubChannel } from "eve/channels/github";

import { githubCredentials } from "../lib/github.js";
import { recordWorkspacePullRequestState } from "../lib/workspace-store.js";

export default githubChannel({
  botName: process.env.GITHUB_APP_SLUG ?? "turborepo-eve-agent",
  credentials: githubCredentials,
  async onPullRequest(ctx, pullRequest) {
    if (ctx.repository.fullName !== "vercel/turborepo") return null;
    const state =
      pullRequest.action === "closed"
        ? pullRequest.raw.merged === true
          ? "merged"
          : "closed"
        : pullRequest.action === "opened" ||
            pullRequest.action === "reopened" ||
            pullRequest.action === "synchronize"
          ? "open"
          : null;
    if (state) {
      await recordWorkspacePullRequestState(
        pullRequest.pullRequestNumber,
        state,
        new Date().toISOString()
      );
    }
    return null;
  }
});
