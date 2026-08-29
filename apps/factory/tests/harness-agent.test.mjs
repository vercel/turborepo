import assert from "node:assert/strict";
import test from "node:test";

import {
  FACTORY_HARNESS_PORT,
  FACTORY_HARNESS_WORKDIR
} from "../agent/lib/harness-agent-config.ts";
import { parseHarnessResumeState } from "../agent/lib/harness-agent-state.ts";

test("HarnessAgent uses the exposed Factory sandbox port and checkout", () => {
  assert.equal(FACTORY_HARNESS_PORT, 4000);
  assert.equal(FACTORY_HARNESS_WORKDIR, "turborepo");
});

test("HarnessAgent resume state rejects malformed persisted values", () => {
  const state = { harnessId: "fx", lifecycleState: { sessionId: "fx_123" } };
  assert.deepEqual(parseHarnessResumeState(JSON.stringify(state)), state);
  assert.equal(parseHarnessResumeState("not json"), undefined);
  assert.equal(parseHarnessResumeState("null"), undefined);
  assert.equal(parseHarnessResumeState('"state"'), undefined);
});
