import assert from "node:assert/strict";
import { spawnSync } from "node:child_process";
import { readFileSync } from "node:fs";
import test from "node:test";

import { JOBS, validateSummary } from "./validate-ci-summary.mjs";

function results({ release = false, eventName = "pull_request" } = {}) {
  const needs = Object.fromEntries(
    JOBS.map((job) => [job, { result: "success" }])
  );
  needs["release-pr"].outputs = { "is-release-pr": String(release) };
  if (release) {
    for (const job of [
      "js_tests",
      "turbo_types_check",
      "rust_test",
      "check-examples",
      "check-lockfiles",
      "js_native_packages"
    ]) {
      needs[job].result = "skipped";
    }
  }
  if (eventName === "push") {
    for (const job of ["quality", "js_tests", "review_gate"]) {
      needs[job].result = "skipped";
    }
  } else {
    needs["check-examples"].result = "skipped";
  }
  return needs;
}

for (const eventName of ["pull_request", "pull_request_review", "push"]) {
  test(`accepts successful CI on ${eventName}`, () => {
    validateSummary(results({ eventName }), eventName);
  });
  for (const job of JOBS) {
    test(`rejects failed, cancelled, or missing ${job} on ${eventName}`, () => {
      for (const result of ["failure", "cancelled", undefined]) {
        const needs = results({ eventName });
        needs[job].result = result;
        assert.throws(() => validateSummary(needs, eventName), new RegExp(job));
      }
    });
  }
}

for (const eventName of ["pull_request", "pull_request_review"]) {
  test(`accepts validated release skips on ${eventName}`, () => {
    validateSummary(results({ release: true }), eventName);
  });
  test(`release skips cannot hide mandatory gate failures on ${eventName}`, () => {
    for (const job of ["release-pr", "quality", "review_gate"]) {
      for (const result of ["failure", "cancelled", "skipped"]) {
        const needs = results({ release: true });
        needs[job].result = result;
        assert.throws(() => validateSummary(needs, eventName), new RegExp(job));
      }
    }
  });
  test(`rejects unexpected skips on ${eventName}`, () => {
    for (const job of JOBS.filter((name) => name !== "check-examples")) {
      const needs = results();
      needs[job].result = "skipped";
      assert.throws(() => validateSummary(needs, eventName), new RegExp(job));
    }
  });
}

test("main pushes cannot skip tests or examples", () => {
  for (const job of [
    "release-pr",
    "turbo_types_check",
    "rust_test",
    "check-examples",
    "check-lockfiles",
    "js_native_packages"
  ]) {
    const needs = results({ eventName: "push" });
    needs[job].result = "skipped";
    assert.throws(() => validateSummary(needs, "push"), new RegExp(job));
  }
});

test("rejects invalid release outputs and missing validation", () => {
  for (const release of [undefined, "", "maybe", true]) {
    const needs = results();
    needs["release-pr"].outputs["is-release-pr"] = release;
    assert.throws(
      () => validateSummary(needs, "pull_request"),
      /validation output/
    );
  }
  const needs = results({ eventName: "push" });
  needs["release-pr"].outputs["is-release-pr"] = "true";
  assert.throws(() => validateSummary(needs, "push"), /validation output/);
  delete needs["release-pr"];
  assert.throws(() => validateSummary(needs, "push"), /validation output/);
});

test("rejects unexpected events, dependencies, and missing results", () => {
  assert.throws(
    () => validateSummary(results(), "workflow_dispatch"),
    /Unexpected CI event/
  );
  assert.throws(
    () =>
      validateSummary(
        { ...results(), extra: { result: "skipped" } },
        "pull_request"
      ),
    /Unexpected CI dependency/
  );
  for (const needs of [null, undefined, []]) {
    assert.throws(
      () => validateSummary(needs, "pull_request"),
      /Missing CI dependency results/
    );
  }
});

test("dependency policy matches the workflow's complete summary needs", () => {
  const workflow = readFileSync(
    new URL("../.github/workflows/turborepo-test.yml", import.meta.url),
    "utf8"
  );
  const block = workflow.match(
    / {2}summary:\n[\s\S]*? {4}needs:\n([\s\S]*?) {4}steps:/
  )?.[1];
  assert.ok(block, "Missing summary dependencies");
  const jobs = [...block.matchAll(/ {6}- (.+)\n/g)].map((match) => match[1]);
  assert.deepEqual(jobs, JOBS);
  const graphJobs = [
    ...workflow.split("jobs:\n")[1].matchAll(/^ {2}([\w-]+):/gm)
  ]
    .map((match) => match[1])
    .filter((job) => !["summary", "cleanup"].includes(job));
  assert.deepEqual(graphJobs.toSorted(), JOBS.toSorted());
  assert.match(workflow, /name: CI Summary/);
  assert.match(workflow, / {2}summary:\n[\s\S]*? {4}if: always\(\)/);
});

test("automated releases approve only the consolidated CI workflow", () => {
  for (const name of ["turborepo-release", "turborepo-library-release"]) {
    const workflow = readFileSync(
      new URL(`../.github/workflows/${name}.yml`, import.meta.url),
      "utf8"
    );
    assert.match(workflow, /for WORKFLOW in turborepo-test\.yml; do/);
    assert.doesNotMatch(
      workflow,
      /for WORKFLOW in .*?(release-review-gate|test-js-packages)\.yml/
    );
    assert.match(workflow, /gh pr checks .*--required --watch --fail-fast/);
  }
});

test("CLI passes green CI and exits nonzero for a failed dependency", () => {
  for (const failed of [false, true]) {
    const needs = results();
    if (failed) {
      needs.quality.result = "failure";
    }
    const output = spawnSync(
      process.execPath,
      [new URL("validate-ci-summary.mjs", import.meta.url).pathname],
      {
        env: {
          ...process.env,
          CI_NEEDS: JSON.stringify(needs),
          CI_EVENT_NAME: "pull_request"
        },
        encoding: "utf8"
      }
    );
    assert.equal(output.status, failed ? 1 : 0);
    assert.match(
      failed ? output.stderr : output.stdout,
      failed ? /quality: failure/ : /All required CI jobs passed/
    );
  }
});
