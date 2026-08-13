import assert from "node:assert/strict";
import test from "node:test";

import {
  CLAUDE_FABLE_MODEL,
  GPT_SOL_MODEL,
  selectPerformanceModels
} from "../agent/lib/performance-models.ts";

test("uses GPT Sol to author and Fable to review on even UTC days", () => {
  assert.deepEqual(selectPerformanceModels(new Date("2026-08-12T23:59:00Z")), {
    authorModel: GPT_SOL_MODEL,
    reviewerModel: CLAUDE_FABLE_MODEL,
    reviewer: "fable_performance_reviewer"
  });
});

test("uses Fable to author and GPT Sol to review on odd UTC days", () => {
  assert.deepEqual(selectPerformanceModels(new Date("2026-08-13T00:01:00Z")), {
    authorModel: CLAUDE_FABLE_MODEL,
    reviewerModel: GPT_SOL_MODEL,
    reviewer: "gpt_performance_reviewer"
  });
});

test("uses the UTC day rather than the host timezone", () => {
  assert.equal(
    selectPerformanceModels(new Date("2026-08-12T23:30:00-07:00")).authorModel,
    CLAUDE_FABLE_MODEL
  );
});
