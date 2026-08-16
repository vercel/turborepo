import assert from "node:assert/strict";
import test from "node:test";

import {
  signWorkflowRun,
  verifyWorkflowRun
} from "../agent/lib/operator-runs.ts";

test("workflow run capabilities bind a run to its OpenCode session", () => {
  const token = signWorkflowRun(
    { sessionID: "ses_eve_one", workflowRunID: "wrun_one" },
    "test-secret"
  );
  assert.equal(verifyWorkflowRun(token, "ses_eve_one", "test-secret"), "wrun_one");
  assert.equal(verifyWorkflowRun(token, "ses_eve_two", "test-secret"), null);
  assert.equal(verifyWorkflowRun(token, "ses_eve_one", "wrong-secret"), null);
});
