import { vercelOidc } from "eve/channels/auth";
import { defaultGitHubAuth, githubChannel } from "eve/channels/github";

import {
  FACTORY_PULL_REQUEST_BRANCH_ATTRIBUTE,
  FACTORY_PULL_REQUEST_FEEDBACK_ATTRIBUTE,
  hasGitHubInvocation,
  isTrustedFactoryPullRequestFeedback
} from "../lib/github-feedback.js";
import { githubCredentials } from "../lib/github.js";
import {
  formatMergedPullRequestSlackNotification,
  mergedFactoryPullRequest
} from "../lib/pull-request.js";
import { markPullRequestSlackNotificationMerged } from "../lib/slack.js";

type PullRequestResponse = { head?: { ref?: string } };
type PermissionResponse = { permission?: string };

const botName = process.env.GITHUB_APP_SLUG ?? "turborepo-eve-agent";

export default githubChannel({
  botName,
  credentials: { ...githubCredentials, webhookVerifier: vercelOidc() },
  turnPolicy: "queue",
  async onPullRequest(ctx, pullRequest) {
    if (ctx.repository.fullName !== "vercel/turborepo") return null;
    const merged = mergedFactoryPullRequest(
      pullRequest.action,
      pullRequest.raw
    );
    if (merged === null) return null;

    try {
      const updated = await markPullRequestSlackNotificationMerged(
        pullRequest.pullRequestNumber,
        formatMergedPullRequestSlackNotification(merged.title, merged.url)
      );
      if (!updated) {
        console.warn("Could not find the Factory pull request Slack message.", {
          pullRequestNumber: pullRequest.pullRequestNumber
        });
      }
    } catch (error) {
      console.warn("Could not update the Factory pull request Slack message.", {
        error,
        pullRequestNumber: pullRequest.pullRequestNumber
      });
    }
    return null;
  },
  async onComment(ctx, comment) {
    if (ctx.conversation.kind !== "review_thread") return null;

    const defaultMentionDispatch = () =>
      hasGitHubInvocation(comment.body, botName)
        ? { auth: defaultGitHubAuth(ctx) }
        : null;
    const pullRequestNumber = ctx.conversation.pullRequestNumber;
    if (
      ctx.repository.fullName !== "vercel/turborepo" ||
      pullRequestNumber === null ||
      ctx.sender.type === "Bot"
    ) {
      return defaultMentionDispatch();
    }

    try {
      const [pullRequest, collaborator] = await Promise.all([
        ctx.github.request<PullRequestResponse>({
          method: "GET",
          path: `/repos/${ctx.repository.fullName}/pulls/${pullRequestNumber}`
        }),
        ctx.github.request<PermissionResponse>({
          method: "GET",
          path: `/repos/${ctx.repository.fullName}/collaborators/${encodeURIComponent(ctx.sender.login)}/permission`
        })
      ]);
      const branch = pullRequest.body.head?.ref;
      if (typeof branch !== "string") return defaultMentionDispatch();
      if (
        !isTrustedFactoryPullRequestFeedback({
          branch,
          conversationKind: ctx.conversation.kind,
          permission: collaborator.body.permission,
          pullRequestNumber,
          repository: ctx.repository.fullName,
          senderType: ctx.sender.type
        })
      ) {
        return defaultMentionDispatch();
      }

      const baseAuth = defaultGitHubAuth(ctx);
      return {
        auth: {
          ...baseAuth,
          attributes: {
            ...baseAuth.attributes,
            [FACTORY_PULL_REQUEST_BRANCH_ATTRIBUTE]: branch,
            [FACTORY_PULL_REQUEST_FEEDBACK_ATTRIBUTE]: "true"
          }
        },
        context: [
          `This is trusted maintainer feedback on Factory pull request #${pullRequestNumber} (${branch}). Read and answer the comment. If it requests code changes, make them in the checked-out PR, run relevant validation, and update this same branch with create_pull_request. Do not create a separate pull request.`
        ],
        title: `Handle feedback on Factory PR #${pullRequestNumber}`
      };
    } catch (error) {
      console.warn("Could not authorize Factory pull request feedback.", error);
      return defaultMentionDispatch();
    }
  }
});
