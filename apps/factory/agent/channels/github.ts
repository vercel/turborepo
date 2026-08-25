import { vercelOidc } from "eve/channels/auth";
import { defaultGitHubAuth, githubChannel } from "eve/channels/github";

import {
  FACTORY_PULL_REQUEST_BRANCH_ATTRIBUTE,
  FACTORY_PULL_REQUEST_FEEDBACK_ATTRIBUTE,
  hasGitHubInvocation,
  isTrustedFactoryPullRequestFeedback
} from "../lib/github-feedback.js";
import { githubCredentials } from "../lib/github.js";
import { FACTORY_ISSUE_ATTRIBUTE } from "../lib/issue-handling.js";

type PullRequestResponse = { head?: { ref?: string } };
type PermissionResponse = { permission?: string };

const botName = process.env.GITHUB_APP_SLUG ?? "turborepo-eve-agent";

export default githubChannel({
  botName,
  credentials: { ...githubCredentials, webhookVerifier: vercelOidc() },
  turnPolicy: "queue",
  onIssue(ctx, issue) {
    if (
      issue.action !== "opened" ||
      ctx.repository.fullName !== "vercel/turborepo" ||
      ctx.sender.type === "Bot"
    ) {
      return null;
    }
    const auth = defaultGitHubAuth(ctx);
    return {
      auth: {
        ...auth,
        attributes: {
          ...auth.attributes,
          [FACTORY_ISSUE_ATTRIBUTE]: "true"
        }
      },
      context: [
        "This session was automatically opened for a new public issue. Follow the Automatic Issue Handling policy exactly."
      ],
      title: "Handle newly opened Turborepo issue"
    };
  },
  async onComment(ctx, comment) {
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
