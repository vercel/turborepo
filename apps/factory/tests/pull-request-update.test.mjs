import assert from "node:assert/strict";
import test from "node:test";

import { resolveExistingPullRequestUpdate } from "../agent/lib/pull-request-update.ts";

const current = {
  checkoutSha: "a".repeat(40),
  currentTreeSha: "b".repeat(40),
  headSha: "a".repeat(40),
  newTreeSha: "c".repeat(40),
  pullRequestUrl: "https://github.com/vercel/turborepo/pull/123"
};

test("updates an existing PR only from its checked-out head", () => {
  assert.equal(resolveExistingPullRequestUpdate(current), "update");
});

test("recognizes an idempotent existing PR publication", () => {
  assert.equal(
    resolveExistingPullRequestUpdate({
      ...current,
      currentTreeSha: current.newTreeSha,
      headSha: "d".repeat(40)
    }),
    "unchanged"
  );
});

test("rejects stale feedback updates instead of overwriting the PR", () => {
  assert.throws(
    () =>
      resolveExistingPullRequestUpdate({
        ...current,
        headSha: "d".repeat(40)
      }),
    /changed after this checkout/
  );
});
