import assert from "node:assert/strict";
import test from "node:test";

import { fxExampleMaintenancePrompt } from "../agent/lib/daily-example-maintenance.ts";

test("fx maintenance scopes work and its pull request branch", () => {
  const prompt = fxExampleMaintenancePrompt("with-nextjs", "run_one");
  assert.match(prompt, /examples\/with-nextjs/);
  assert.match(prompt, /examples-with-nextjs-fx-run_one/);
  assert.match(prompt, /Do not list routine validation that CI will run/);
  assert.match(prompt, /non-routine manual testing/);
  assert.doesNotMatch(prompt, /summary and validation results/);
  assert.doesNotMatch(
    prompt,
    /select_daily_example|run_example_turbo_tasks|create_pull_request/
  );
});
