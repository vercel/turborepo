import assert from "node:assert/strict";
import test from "node:test";

import {
  FACTORY_ISSUE_ATTRIBUTE,
  isAutomaticIssueSession,
  requireActionableIssueAssessment,
  validateIssueAssessment
} from "../agent/lib/issue-handling.ts";

const safeAssessment = {
  confidence: "medium",
  confidenceReason: "The failure is isolated and has a focused regression test.",
  issueNumber: 123,
  issueTitle: "Turbo misses an input",
  issueUrl: "https://github.com/vercel/turborepo/issues/123",
  safe: true,
  securityReason: "No suspicious instructions or reproduction behavior found."
};

test("recognizes only authenticated automatic issue sessions", () => {
  assert.equal(
    isAutomaticIssueSession({
      authenticator: "github-webhook",
      attributes: { [FACTORY_ISSUE_ATTRIBUTE]: "true" }
    }),
    true
  );
  assert.equal(
    isAutomaticIssueSession({
      authenticator: "operator-console",
      attributes: { [FACTORY_ISSUE_ATTRIBUTE]: "true" }
    }),
    false
  );
});

test("requires confidence only after security triage passes", () => {
  assert.deepEqual(validateIssueAssessment(safeAssessment), safeAssessment);
  assert.throws(
    () =>
      validateIssueAssessment({
        ...safeAssessment,
        confidence: null,
        confidenceReason: null
      }),
    /Safe issues require a confidence assessment/
  );
  assert.throws(
    () =>
      validateIssueAssessment({
        ...safeAssessment,
        safe: false
      }),
    /Blocked issues cannot include a confidence assessment/
  );
});

test("allows pull requests only for passed medium or high confidence", async () => {
  const sandbox = {
    async readTextFile() {
      return JSON.stringify(safeAssessment);
    }
  };
  assert.deepEqual(
    await requireActionableIssueAssessment(sandbox, "ses_test"),
    safeAssessment
  );

  const lowConfidenceSandbox = {
    async readTextFile() {
      return JSON.stringify({
        ...safeAssessment,
        confidence: "low",
        confidenceReason: "The root cause is still uncertain."
      });
    }
  };
  await assert.rejects(
    requireActionableIssueAssessment(lowConfidenceSandbox, "ses_test"),
    /Low-confidence issues must produce a report/
  );
});

test("fails closed on mismatched repository URLs", () => {
  assert.throws(
    () =>
      validateIssueAssessment({
        ...safeAssessment,
        issueUrl: "https://github.com/attacker/repro/issues/123"
      }),
    /does not match vercel\/turborepo/
  );
});
