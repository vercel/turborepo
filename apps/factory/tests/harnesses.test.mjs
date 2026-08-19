import assert from "node:assert/strict";
import test from "node:test";

import { harnessExampleMaintenancePrompt } from "../agent/lib/daily-example-maintenance.ts";
import {
  HARNESS_IDS,
  isHarnessId,
  isSandboxId
} from "../agent/lib/harnesses.ts";

test("only registered harnesses and sandboxes are accepted", () => {
  assert.deepEqual(HARNESS_IDS, ["claude-code", "codex", "opencode"]);
  assert.equal(isHarnessId("codex"), true);
  assert.equal(isHarnessId("unknown"), false);
  assert.equal(isSandboxId("vercel"), true);
  assert.equal(isSandboxId("unknown"), false);
});

test("harness maintenance uses native tools for the selected example", () => {
  const prompt = harnessExampleMaintenancePrompt(
    "with-nextjs",
    "codex",
    "ses_one"
  );
  assert.match(prompt, /examples\/with-nextjs/);
  assert.match(prompt, /examples-with-nextjs-codex-ses_one/);
  assert.doesNotMatch(
    prompt,
    /select_daily_example|run_example_turbo_tasks|create_pull_request/
  );
});
