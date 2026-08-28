import assert from "node:assert/strict";
import test from "node:test";

import {
  isWorkspaceRunning,
  workspaceStatusLabel
} from "../app/workspace-types.ts";

test("workspace status labels describe operator-visible state", () => {
  assert.equal(workspaceStatusLabel({ status: "idle" }), "Ready");
  assert.equal(workspaceStatusLabel({ status: "running" }), "Working");
  assert.equal(
    workspaceStatusLabel({ activity: "Running tests", status: "running" }),
    "Running tests"
  );
  assert.equal(
    workspaceStatusLabel({
      pullRequest: { number: 123, state: "open" },
      status: "idle"
    }),
    "PR open"
  );
  assert.equal(workspaceStatusLabel({ status: "done" }), "Done");
  assert.equal(workspaceStatusLabel({ status: "error" }), "Error");
  assert.equal(isWorkspaceRunning("running"), true);
  assert.equal(isWorkspaceRunning("idle"), false);
});
