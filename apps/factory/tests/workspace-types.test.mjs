import assert from "node:assert/strict";
import test from "node:test";

import {
  isWorkspaceRunning,
  workspaceStatusLabel
} from "../app/workspace-types.ts";

test("workspace status labels describe operator-visible state", () => {
  assert.equal(workspaceStatusLabel("idle"), "Ready");
  assert.equal(workspaceStatusLabel("running"), "Working");
  assert.equal(workspaceStatusLabel("pending"), "Working");
  assert.equal(workspaceStatusLabel("error"), "Error");
  assert.equal(isWorkspaceRunning("running"), true);
  assert.equal(isWorkspaceRunning("idle"), false);
});
