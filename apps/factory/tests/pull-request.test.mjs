import assert from "node:assert/strict";
import test from "node:test";

import { buildDraftPullRequest } from "../agent/lib/pull-request.ts";

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
