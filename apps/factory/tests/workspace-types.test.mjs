import assert from "node:assert/strict";
import test from "node:test";

import {
  isWorkspaceRunning,
  latestWorkspaceFailure,
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

test("projects actionable failure information from workspace events", () => {
  const events = [
    { type: "turn.started", data: { turnId: "turn_1" } },
    {
      type: "step.failed",
      data: {
        code: "gateway-auth",
        message: "Failed",
        details: {
          hint: "Refresh the AI Gateway credentials.",
          detail: "401 Unauthorized\nCaused by an expired token."
        }
      }
    },
    {
      type: "turn.failed",
      data: {
        code: "gateway-auth",
        message: "Failed",
        details: {
          hint: "Refresh the AI Gateway credentials.",
          detail: "401 Unauthorized\nCaused by an expired token."
        }
      }
    }
  ];
  assert.deepEqual(latestWorkspaceFailure(events), {
    code: "gateway-auth",
    message: "Failed",
    hint: "Refresh the AI Gateway credentials.",
    detail: "401 Unauthorized\nCaused by an expired token."
  });
});

test("clears an old workspace failure when a later turn starts", () => {
  assert.equal(
    latestWorkspaceFailure([
      {
        type: "session.failed",
        data: { code: "failed", message: "First run failed" }
      },
      { type: "turn.started", data: { turnId: "turn_2" } }
    ]),
    undefined
  );
});

test("ignores malformed workspace failure events", () => {
  assert.equal(
    latestWorkspaceFailure([
      { type: "turn.failed", data: { code: "failed", message: "  " } }
    ]),
    undefined
  );
});
