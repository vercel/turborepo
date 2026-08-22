import assert from "node:assert/strict";
import test from "node:test";

import {
  buildDraftPullRequest,
  resolvePullRequestTitle
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
