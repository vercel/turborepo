import assert from "node:assert/strict";
import test from "node:test";

import { run } from "./validate-review-gate.mjs";

const ENV_NAMES = [
  "GH_TOKEN",
  "GITHUB_REPOSITORY",
  "GITHUB_SHA",
  "PR_NUMBER",
  "EXPECTED_HEAD_SHA",
];

async function withGitHub(
  { files, filePages, reviews = [], permission = "write" },
  fn,
) {
  const originalFetch = globalThis.fetch;
  const originalEnv = Object.fromEntries(
    ENV_NAMES.map((name) => [name, process.env[name]]),
  );
  const pull = {
    number: 42,
    title: "feat: Test change",
    state: "open",
    draft: false,
    user: { id: 1, login: "author" },
    base: {
      ref: "main",
      sha: "base-sha",
      repo: { default_branch: "main", full_name: "vercel/turborepo" },
    },
    head: {
      ref: "feature",
      sha: "head-sha",
      repo: { full_name: "vercel/turborepo" },
    },
  };

  globalThis.fetch = async (url) => {
    const parsed = new URL(url);
    const path = parsed.pathname;
    if (path.endsWith("/pulls/42")) {
      return response(pull);
    }
    if (path.endsWith("/pulls/42/files")) {
      const page = Number(parsed.searchParams.get("page"));
      return response(filePages?.[page - 1] ?? files);
    }
    if (path.endsWith("/pulls/42/reviews")) {
      return response(reviews);
    }
    if (path.includes("/collaborators/")) {
      return response({ permission });
    }
    throw new Error(`Unexpected request: ${path}`);
  };
  Object.assign(process.env, {
    GH_TOKEN: "test-token",
    GITHUB_REPOSITORY: "vercel/turborepo",
    GITHUB_SHA: "head-sha",
    PR_NUMBER: "42",
    EXPECTED_HEAD_SHA: "head-sha",
  });

  try {
    await fn(pull);
  } finally {
    globalThis.fetch = originalFetch;
    for (const [name, value] of Object.entries(originalEnv)) {
      if (value === undefined) {
        delete process.env[name];
      } else {
        process.env[name] = value;
      }
    }
  }
}

test("defers non-release paths to the native team review policy", async () => {
  await withGitHub(
    { files: [{ filename: "crates/turborepo-lib/src/lib.rs" }] },
    async () => assert.doesNotReject(run()),
  );
});

test("defers fork PRs with non-release paths to native review", async () => {
  await withGitHub(
    { files: [{ filename: "crates/turborepo-lib/src/lib.rs" }] },
    async (pull) => {
      pull.head.repo.full_name = "contributor/turborepo";
      await assert.doesNotReject(run());
    },
  );
});

test("finds non-release paths after the first page", async () => {
  await withGitHub(
    {
      filePages: [
        Array.from({ length: 100 }, () => ({ filename: "version.txt" })),
        [{ filename: "crates/turborepo-lib/src/lib.rs" }],
      ],
    },
    async () => assert.doesNotReject(run()),
  );
});

test("requires human approval for non-release changes to exempt paths", async () => {
  await withGitHub(
    { files: [{ filename: "packages/turbo/package.json" }] },
    async () =>
      assert.rejects(
        run(),
        /write-authorized human must approve non-release changes/,
      ),
  );
});

test("accepts a write-authorized approval for exempt paths", async () => {
  await withGitHub(
    {
      files: [{ filename: "packages/turbo/package.json" }],
      reviews: [
        {
          id: 1,
          commit_id: "head-sha",
          state: "APPROVED",
          user: { login: "reviewer", type: "User" },
        },
      ],
    },
    async () => assert.doesNotReject(run()),
  );
});

test("rejects requested changes from a write-authorized reviewer", async () => {
  await withGitHub(
    {
      files: [{ filename: "version.txt" }],
      reviews: [
        {
          id: 1,
          commit_id: "head-sha",
          state: "CHANGES_REQUESTED",
          user: { login: "reviewer", type: "User" },
        },
      ],
    },
    async () => assert.rejects(run(), /Changes are requested by reviewer/),
  );
});

test("rejects a stale approval for exempt paths", async () => {
  await withGitHub(
    {
      files: [{ filename: "version.txt" }],
      reviews: [
        {
          id: 1,
          commit_id: "old-sha",
          state: "APPROVED",
          user: { login: "reviewer", type: "User" },
        },
      ],
    },
    async () =>
      assert.rejects(
        run(),
        /write-authorized human must approve non-release changes/,
      ),
  );
});

test("rejects a dispatch whose workflow SHA differs from the PR head", async () => {
  await withGitHub(
    { files: [{ filename: "version.txt" }] },
    async () => {
      process.env.GITHUB_SHA = "different-sha";
      await assert.rejects(run(), /not running on the dispatched SHA/);
    },
  );
});

function response(data) {
  return {
    ok: true,
    async json() {
      return data;
    },
  };
}
