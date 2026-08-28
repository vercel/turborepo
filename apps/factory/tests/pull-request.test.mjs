import assert from "node:assert/strict";
import test from "node:test";

import {
  buildBranchRefUpdate,
  buildDraftPullRequest,
  resolvePullRequestTitle,
  updateBranchRefWithLease
} from "../agent/lib/pull-request.ts";

test("builds draft pull requests", () => {
  assert.deepEqual(
    buildDraftPullRequest({
      title: "chore: Update Turborepo examples",
      body: "Maintenance update",
      head: "agents/examples-update",
      base: "main"
    }),
    {
      title: "chore: Update Turborepo examples",
      body: "Maintenance update",
      head: "agents/examples-update",
      base: "main",
      draft: true
    }
  );
});

test("builds force-with-lease branch updates", () => {
  assert.deepEqual(
    buildBranchRefUpdate("agents/update", "expected-head", "new-head"),
    {
      afterOid: "new-head",
      beforeOid: "expected-head",
      force: true,
      name: "refs/heads/agents/update"
    }
  );
});

test("updates a branch with an atomic expected-head lease", async () => {
  let request;
  await updateBranchRefWithLease(
    {
      branchName: "agents/update",
      expectedSha: "expected-head",
      newSha: "new-head",
      repositoryId: "repository-id",
      token: "installation-token"
    },
    async (url, init) => {
      request = { url, init };
      return Response.json({
        data: { updateRefs: { clientMutationId: null } }
      });
    }
  );

  assert.equal(request.url, "https://api.github.com/graphql");
  assert.equal(request.init.method, "POST");
  assert.equal(request.init.headers.authorization, "Bearer installation-token");
  const body = JSON.parse(request.init.body);
  assert.match(body.query, /updateRefs/);
  assert.deepEqual(body.variables, {
    input: {
      repositoryId: "repository-id",
      refUpdates: [
        {
          afterOid: "new-head",
          beforeOid: "expected-head",
          force: true,
          name: "refs/heads/agents/update"
        }
      ]
    }
  });
});

test("rejects concurrent branch updates and malformed GraphQL success", async () => {
  const input = {
    branchName: "agents/update",
    expectedSha: "expected-head",
    newSha: "new-head",
    repositoryId: "repository-id",
    token: "installation-token"
  };
  await assert.rejects(
    updateBranchRefWithLease(input, async () =>
      Response.json({
        errors: [{ message: "Expected ref to point to expected-head" }]
      })
    ),
    /Expected ref to point to expected-head/
  );
  await assert.rejects(
    updateBranchRefWithLease(input, async () => Response.json({ data: {} })),
    /did not confirm the ref update/
  );
});

test("titles automated example maintenance from its selection", () => {
  assert.equal(
    resolvePullRequestTitle({
      automatedExample: "with-svelte",
      requestedTitle: "feat: Ignored"
    }),
    "chore: Update with-svelte example"
  );
});

test("requires a perf title for an automated performance run", () => {
  assert.equal(
    resolvePullRequestTitle({
      performance: true,
      requestedTitle: "perf: Hash tasks once per package"
    }),
    "perf: Hash tasks once per package"
  );
  assert.throws(
    () =>
      resolvePullRequestTitle({
        performance: true,
        requestedTitle: "fix: Hash tasks once per package"
      }),
    /perf: Description/
  );
  assert.throws(() => resolvePullRequestTitle({ performance: true }), /perf/);
});

test("requires a conventional title for an ad-hoc run", () => {
  assert.equal(
    resolvePullRequestTitle({ requestedTitle: "fix: Show invalid task globs" }),
    "fix: Show invalid task globs"
  );
  assert.throws(() => resolvePullRequestTitle({}), /Conventional Commit/);
  assert.throws(
    () => resolvePullRequestTitle({ requestedTitle: "fix: lowercase" }),
    /Conventional Commit/
  );
  assert.throws(
    () => resolvePullRequestTitle({ requestedTitle: "fix(run): Scoped" }),
    /Conventional Commit/
  );
});
