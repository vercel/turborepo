import assert from "node:assert/strict";
import test from "node:test";

import {
  CLAUDE_OPUS_MODEL,
  GPT_SOL_MODEL,
  selectPerformanceModels
} from "../agent/lib/performance-models.ts";

test("uses Sol 6.1 and Claude Opus 5.5", () => {
  assert.equal(GPT_SOL_MODEL, "openai/gpt-6.1-sol");
  assert.equal(CLAUDE_OPUS_MODEL, "anthropic/claude-opus-5.5");
});

test("uses GPT Sol to author and Opus to review on even UTC days", () => {
  assert.deepEqual(selectPerformanceModels(new Date("2026-08-12T23:59:00Z")), {
    authorModel: GPT_SOL_MODEL,
    reviewerModel: CLAUDE_OPUS_MODEL,
    reviewer: "fable_performance_reviewer"
  });
});

test("uses Opus to author and GPT Sol to review on odd UTC days", () => {
  assert.deepEqual(selectPerformanceModels(new Date("2026-08-13T00:01:00Z")), {
    authorModel: CLAUDE_OPUS_MODEL,
    reviewerModel: GPT_SOL_MODEL,
    reviewer: "gpt_performance_reviewer"
  });
});

test("uses the UTC day rather than the host timezone", () => {
  assert.equal(
    selectPerformanceModels(new Date("2026-08-12T23:30:00-07:00")).authorModel,
    CLAUDE_OPUS_MODEL
  );
});
