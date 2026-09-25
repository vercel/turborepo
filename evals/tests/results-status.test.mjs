import assert from "node:assert/strict";
import { mkdirSync, mkdtempSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import test from "node:test";
import { ungradedAttempts } from "../lib/results-status.mjs";

function record(root, timestamp, variant, fixture, result) {
  const path = join(root, variant, timestamp, fixture);
  mkdirSync(join(path, "run-1"), { recursive: true });
  writeFileSync(join(path, "summary.json"), "{}\n");
  writeFileSync(join(path, "run-1/result.json"), JSON.stringify(result));
}

test("ungraded failures are flagged, but a newer graded result supersedes them", (t) => {
  const root = mkdtempSync(join(tmpdir(), "turbo-eval-results-"));
  t.after(() => rmSync(root, { recursive: true, force: true }));
  record(root, "2026-09-25T01:00:00Z", "baseline", "env-cache", {
    status: "failed",
    error: "API Error: 402 User budget exceeded"
  });
  assert.deepEqual(
    ungradedAttempts(root, ["baseline", "agents-md"], ["env-cache"]),
    ["baseline/env-cache"]
  );
  assert.deepEqual(ungradedAttempts(root, ["agents-md"], ["env-cache"]), []);

  record(root, "2026-09-25T02:00:00Z", "baseline", "env-cache", {
    status: "failed",
    outputPaths: { eval: "./outputs/eval.txt" }
  });
  assert.deepEqual(ungradedAttempts(root, ["baseline"], ["env-cache"]), []);
});
