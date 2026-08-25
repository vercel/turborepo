import assert from "node:assert/strict";
import test from "node:test";

import {
  FACTORY_PULL_REQUEST_BRANCH_ATTRIBUTE,
  FACTORY_PULL_REQUEST_FEEDBACK_ATTRIBUTE,
  hasGitHubInvocation,
  isAuthorizedFactoryPullRequestUpdate,
  isTrustedFactoryPullRequestFeedback
} from "../agent/lib/github-feedback.ts";

const trusted = {
  branch: "agents/examples-basic-2026-08-25",
  conversationKind: "pull_request",
  permission: "write",
  pullRequestNumber: 123,
  repository: "vercel/turborepo",
  senderType: "User"
};

test("accepts trusted feedback on Factory pull requests", () => {
  assert.equal(isTrustedFactoryPullRequestFeedback(trusted), true);
  assert.equal(
    isTrustedFactoryPullRequestFeedback({
      ...trusted,
      conversationKind: "review_thread",
      permission: "admin"
    }),
    true
  );
});

test("rejects comments that cannot safely drive Factory changes", () => {
  for (const candidate of [
    { ...trusted, branch: "feature/not-factory" },
    { ...trusted, conversationKind: "issue" },
    { ...trusted, permission: "read" },
    { ...trusted, pullRequestNumber: null },
    { ...trusted, repository: "someone/fork" },
    { ...trusted, senderType: "Bot" }
  ]) {
    assert.equal(isTrustedFactoryPullRequestFeedback(candidate), false);
  }
});

test("authorizes updates only for the branch authenticated by the webhook", () => {
  const auth = {
    authenticator: "github-webhook",
    attributes: {
      [FACTORY_PULL_REQUEST_BRANCH_ATTRIBUTE]: trusted.branch,
      [FACTORY_PULL_REQUEST_FEEDBACK_ATTRIBUTE]: "true"
    }
  };
  assert.equal(
    isAuthorizedFactoryPullRequestUpdate(auth, trusted.branch),
    true
  );
  assert.equal(
    isAuthorizedFactoryPullRequestUpdate(auth, "agents/another-pr"),
    false
  );
  assert.equal(
    isAuthorizedFactoryPullRequestUpdate(
      { ...auth, authenticator: "operator-console" },
      trusted.branch
    ),
    false
  );
  assert.equal(
    isAuthorizedFactoryPullRequestUpdate(
      {
        ...auth,
        attributes: {
          [FACTORY_PULL_REQUEST_BRANCH_ATTRIBUTE]: trusted.branch
        }
      },
      trusted.branch
    ),
    false
  );
});

test("preserves explicit GitHub invocation outside automatic feedback", () => {
  assert.equal(
    hasGitHubInvocation(
      "please @turbo.factory-agent review this",
      "turbo.factory-agent"
    ),
    true
  );
  assert.equal(
    hasGitHubInvocation("@turbo.factory-agent-extra no", "turbo.factory-agent"),
    false
  );
  assert.equal(hasGitHubInvocation("ordinary comment", "factory-agent"), false);
});
