import assert from "node:assert/strict";
import test from "node:test";

import {
  isWorkspaceRunning,
  workspaceStatusLabel
} from "../app/workspace-status-types.ts";

test("workspace display statuses include activity and pull request state", () => {
  assert.equal(workspaceStatusLabel({ status: "idle" }), "Ready");
  assert.equal(
    workspaceStatusLabel({ activity: "Running tests", status: "running" }),
    "Running tests"
  );
  assert.equal(
    workspaceStatusLabel({
      pullRequest: {
        number: 123,
        state: "open",
        url: "https://github.com/vercel/turborepo/pull/123"
      },
      status: "idle"
    }),
    "PR open"
  );
  assert.equal(workspaceStatusLabel({ status: "done" }), "Done");
  assert.equal(isWorkspaceRunning("running"), true);
  assert.equal(isWorkspaceRunning("idle"), false);
});
