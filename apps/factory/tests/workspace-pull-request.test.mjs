import assert from "node:assert/strict";
import test from "node:test";

import {
  githubPullRequestState,
  shouldRefreshWorkspacePullRequest,
  workspaceSandboxName
} from "../agent/lib/workspace.ts";

function workspace(pullRequest) {
  return {
    agent: "fx",
    createdAt: "2026-08-22T12:00:00.000Z",
    id: "ws_abc",
    messages: [],
    pullRequest,
    sandbox: {
      name: workspaceSandboxName("ws_abc"),
      provider: "vercel",
      status: "running"
    },
    status: "idle",
    title: "Fix caching",
    updatedAt: "2026-08-22T12:00:00.000Z",
    version: 1
  };
}

test("normalizes GitHub pull request state", () => {
  assert.equal(
    githubPullRequestState({ merged_at: "2026-08-22T12:00:00Z", state: "closed" }),
    "merged"
  );
  assert.equal(githubPullRequestState({ merged_at: null, state: "open" }), "open");
  assert.equal(
    githubPullRequestState({ merged_at: null, state: "closed" }),
    "closed"
  );
  assert.equal(githubPullRequestState({ state: "unknown" }), null);
});

test("refreshes legacy and stale open pull request records", () => {
  const now = new Date("2026-08-22T12:20:00.000Z");
  assert.equal(
    shouldRefreshWorkspacePullRequest(
      workspace({
        number: 123,
        url: "https://github.com/vercel/turborepo/pull/123"
      }),
      now
    ),
    true
  );
  assert.equal(
    shouldRefreshWorkspacePullRequest(
      workspace({
        checkedAt: "2026-08-22T12:00:00.000Z",
        number: 123,
        state: "open",
        url: "https://github.com/vercel/turborepo/pull/123"
      }),
      now
    ),
    true
  );
  assert.equal(
    shouldRefreshWorkspacePullRequest(
      workspace({
        checkedAt: "2026-08-22T12:15:00.000Z",
        number: 123,
        state: "open",
        url: "https://github.com/vercel/turborepo/pull/123"
      }),
      now
    ),
    false
  );
  assert.equal(
    shouldRefreshWorkspacePullRequest(
      workspace({
        checkedAt: "2026-08-22T12:00:00.000Z",
        number: 123,
        state: "closed",
        url: "https://github.com/vercel/turborepo/pull/123"
      }),
      now
    ),
    true
  );
  assert.equal(
    shouldRefreshWorkspacePullRequest(
      workspace({
        checkedAt: "2026-08-22T12:00:00.000Z",
        number: 123,
        state: "merged",
        url: "https://github.com/vercel/turborepo/pull/123"
      }),
      now
    ),
    false
  );
});
