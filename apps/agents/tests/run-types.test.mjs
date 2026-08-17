import assert from "node:assert/strict";
import test from "node:test";

import { isAgentRunRecord } from "../agent/lib/run-types.ts";

test("validates normalized agent run records", () => {
  const run = {
    agent: "codex",
    harness: "codex",
    id: "ses_one",
    source: "harness",
    startedAt: "2026-08-16T12:00:00.000Z",
    status: "running",
    title: "Maintain example",
    trigger: "operator",
    updatedAt: "2026-08-16T12:00:00.000Z"
  };
  assert.equal(isAgentRunRecord(run), true);
  assert.equal(isAgentRunRecord({ ...run, status: "unknown" }), false);
  assert.equal(isAgentRunRecord(null), false);
});
