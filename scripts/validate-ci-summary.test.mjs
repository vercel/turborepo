import assert from "node:assert/strict";
import { spawnSync } from "node:child_process";
import { readFileSync } from "node:fs";
import test from "node:test";

import {
  JOBS,
  validateFinalSummary,
  validateSummary
} from "./validate-ci-summary.mjs";

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
    for (const job of ["quality", "js_tests"]) {
      needs[job].result = "skipped";
    }
  } else {
    needs["check-examples"].result = "skipped";
  }
  return needs;
}

for (const eventName of ["pull_request", "push"]) {
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

for (const eventName of ["pull_request"]) {
  test(`accepts validated release skips on ${eventName}`, () => {
    validateSummary(results({ release: true }), eventName);
  });
  test(`release skips cannot hide mandatory gate failures on ${eventName}`, () => {
    for (const job of ["release-pr", "quality"]) {
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
  for (const eventName of [
    "pull_request_review",
    "workflow_dispatch",
    undefined
  ]) {
    assert.throws(
      () => validateSummary(results(), eventName),
      /Unexpected CI event/
    );
  }
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

const workflow = readFileSync(
  new URL("../.github/workflows/turborepo-test.yml", import.meta.url),
  "utf8"
);
const workflowJobs = Object.fromEntries(
  [
    ...workflow
      .split("jobs:\n")[1]
      .matchAll(/^ {2}([\w-]+):\n([\s\S]*?)(?=^ {2}[\w-]+:|$(?![\s\S]))/gm)
  ].map((match) => [match[1], match[2]])
);

test("dependency policy matches the workflow's complete code checks needs", () => {
  const block = workflowJobs.code_checks.match(
    / {4}needs:\n([\s\S]*?) {4}steps:/
  )?.[1];
  assert.ok(block, "Missing code checks dependencies");
  const jobs = [...block.matchAll(/ {6}- (.+)\n/g)].map((match) => match[1]);
  assert.deepEqual(jobs, JOBS);
  const graphJobs = Object.keys(workflowJobs).filter(
    (job) => !["code_checks", "summary", "cleanup"].includes(job)
  );
  assert.deepEqual(graphJobs.toSorted(), JOBS.toSorted());
  assert.match(
    workflowJobs.code_checks,
    /name: "Code checks \(PR #\$\{\{ github\.event\.pull_request\.number \|\| 0 \}\}, \$\{\{ github\.event\.pull_request\.base\.ref \|\| github\.ref_name \}\}\)"/
  );
  assert.match(workflowJobs.code_checks, /CI_CODE_ONLY: "true"/);
  assert.match(
    workflowJobs.code_checks,
    /node --test scripts\/validate-ci-summary\.test\.mjs scripts\/validate-review-ci\.test\.mjs\n {10}node scripts\/validate-ci-summary\.mjs/
  );
});

test("code checks and cleanup never run for review events", () => {
  for (const job of [...JOBS, "code_checks", "cleanup"]) {
    const condition = workflowJobs[job].match(/^ {4}if: (.+)$/m)?.[1];
    assert.ok(condition, `Missing condition for ${job}`);
    if (["quality", "js_tests"].includes(job)) {
      assert.match(condition, /github\.event_name == 'pull_request'/);
    } else if (job === "check-examples") {
      assert.match(condition, /github\.event_name == 'push'/);
    } else {
      assert.match(condition, /github\.event_name != 'pull_request_review'/);
    }
    if (["code_checks", "cleanup"].includes(job)) {
      assert.match(condition, /^always\(\) && /);
    }
  }
  assert.match(
    workflowJobs.js_tests,
    /needs\.release-pr\.outputs\.is-release-pr != 'true'/
  );
  assert.match(
    workflowJobs["release-pr"],
    /if \[\[ "\$EVENT_NAME" != "pull_request" \]\]; then/
  );
  assert.doesNotMatch(
    workflowJobs["release-pr"],
    /"\$EVENT_NAME" != "pull_request_review"/
  );
  assert.match(
    workflow,
    /group: .*\$\{\{ github\.event_name == 'pull_request_review' && 'review' \|\| 'code' \}\}/
  );
});

test("final summary validates CI before running trusted review policy", () => {
  const summary = workflowJobs.summary;
  assert.match(summary, /^ {4}name: CI Summary$/m);
  assert.match(summary, /^ {4}if: always\(\)$/m);
  assert.match(summary, /^ {4}timeout-minutes: 35$/m);
  assert.match(summary, /^ {4}needs: code_checks$/m);
  assert.equal((summary.match(/^ {4}needs:/gm) ?? []).length, 1);
  assert.match(
    summary,
    /permissions:\n {6}contents: read\n {6}actions: read\n {6}pull-requests: read/
  );
  assert.doesNotMatch(summary, /CI_CODE_ONLY/);
  for (const variable of [
    "CI_NEEDS",
    "CI_EVENT_NAME",
    "GH_TOKEN",
    "PR_NUMBER",
    "PR_HEAD_SHA"
  ]) {
    assert.match(summary, new RegExp(` {10}${variable}:`));
  }
  const steps = summary.split("      - name: ").slice(1);
  assert.equal(steps.length, 4);
  assert.match(steps[0], /^Checkout\n/);
  assert.match(steps[0], /persist-credentials: false/);
  assert.match(steps[1], /run: node scripts\/validate-ci-summary\.mjs/);
  assert.match(steps[2], /if: github\.event_name != 'push'/);
  assert.match(
    steps[2],
    /ref: \$\{\{ github\.event\.repository\.default_branch \}\}/
  );
  assert.match(steps[2], /path: trusted-review-policy/);
  assert.match(steps[2], /persist-credentials: false/);
  assert.match(steps[3], /if: github\.event_name != 'push'/);
  assert.match(steps[3], /GH_TOKEN: \$\{\{ github\.token \}\}/);
  assert.match(
    steps[3],
    /PR_NUMBER: \$\{\{ github\.event\.pull_request\.number \}\}/
  );
  assert.match(
    steps[3],
    /run: node trusted-review-policy\/scripts\/validate-review-gate\.mjs/
  );
});

for (const eventName of ["pull_request", "pull_request_review", "push"]) {
  const expected = eventName === "pull_request_review" ? "skipped" : "success";
  test(`final summary accepts ${expected} code checks on ${eventName}`, () => {
    validateFinalSummary({ code_checks: { result: expected } }, eventName);
  });
  test(`final summary rejects every other code checks result on ${eventName}`, () => {
    for (const result of [
      "success",
      "skipped",
      "failure",
      "cancelled",
      "pending",
      "",
      null,
      undefined
    ]) {
      if (result === expected) {
        continue;
      }
      assert.throws(
        () => validateFinalSummary({ code_checks: { result } }, eventName),
        /code_checks:/
      );
    }
    for (const needs of [{}, { code_checks: {} }, { code_checks: null }]) {
      assert.throws(
        () => validateFinalSummary(needs, eventName),
        /code_checks: missing/
      );
    }
  });
  test(`final summary rejects unexpected dependencies and missing results on ${eventName}`, () => {
    for (const job of [...JOBS, "summary", "cleanup", "review_gate"]) {
      assert.throws(
        () =>
          validateFinalSummary(
            { code_checks: { result: expected }, [job]: { result: "success" } },
            eventName
          ),
        /Unexpected CI dependency/
      );
    }
    for (const needs of [null, undefined, [], "success", true]) {
      assert.throws(
        () => validateFinalSummary(needs, eventName),
        /Missing CI dependency results/
      );
    }
  });
}

test("final summary rejects unexpected events", () => {
  for (const eventName of [
    "workflow_dispatch",
    "pull_request_target",
    "",
    undefined
  ]) {
    assert.throws(
      () =>
        validateFinalSummary({ code_checks: { result: "success" } }, eventName),
      /Unexpected CI event/
    );
  }
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

function runSummary(needs, eventName, codeOnly = "false") {
  return spawnSync(
    process.execPath,
    [new URL("validate-ci-summary.mjs", import.meta.url).pathname],
    {
      env: {
        ...process.env,
        CI_NEEDS: JSON.stringify(needs),
        CI_EVENT_NAME: eventName,
        CI_CODE_ONLY: codeOnly,
        GH_TOKEN: "",
        GITHUB_REPOSITORY: "",
        GITHUB_RUN_ID: "",
        PR_NUMBER: "",
        PR_HEAD_SHA: "",
        GITHUB_SHA: ""
      },
      encoding: "utf8"
    }
  );
}

test("CLI final summary passes PR and push code checks and fails other results", () => {
  for (const eventName of ["pull_request", "push"]) {
    for (const result of [
      "success",
      "skipped",
      "failure",
      "cancelled",
      undefined
    ]) {
      const output = runSummary({ code_checks: { result } }, eventName);
      assert.equal(output.status, result === "success" ? 0 : 1);
      assert.match(
        result === "success" ? output.stdout : output.stderr,
        result === "success" ? /All required CI jobs passed/ : /code_checks:/
      );
    }
  }
});

test("CLI reviews never pass code-only validation", () => {
  for (const needs of [
    results(),
    results({ release: true }),
    { code_checks: { result: "skipped" } }
  ]) {
    const output = runSummary(needs, "pull_request_review", "true");
    assert.equal(output.status, 1);
    assert.match(output.stderr, /Unexpected CI event: pull_request_review/);
  }
});

test("CLI review rejects invalid code checks before looking up reusable CI", () => {
  for (const result of ["success", "failure", "cancelled", undefined]) {
    const output = runSummary(
      { code_checks: { result } },
      "pull_request_review"
    );
    assert.equal(output.status, 1);
    assert.match(output.stderr, /code_checks:/);
    assert.doesNotMatch(
      output.stdout,
      /Reused code CI run|All required CI jobs passed/
    );
  }
});

test("CLI review reuses CI with the expected context and propagates lookup failures", () => {
  for (const failed of [false, true]) {
    const fixture = `
      import assert from "node:assert/strict";
      export async function validateReviewCI(options) {
        assert.deepEqual(options, {
          repository: "vercel/turborepo",
          token: "test-token",
          runId: 123,
          pullNumber: 456,
          headSha: "head-sha",
          reviewSha: "review-sha"
        });
        ${failed ? 'throw new Error("No reusable code CI run");' : 'return 789;'}
      }
    `;
    const moduleURL = `data:text/javascript,${encodeURIComponent(fixture)}`;
    const loader = `
      import { registerHooks } from "node:module";
      registerHooks({
        resolve(specifier, context, nextResolve) {
          if (specifier === "./validate-review-ci.mjs") {
            return { url: ${JSON.stringify(moduleURL)}, shortCircuit: true };
          }
          return nextResolve(specifier, context);
        }
      });
    `;
    const output = spawnSync(
      process.execPath,
      [
        "--import",
        `data:text/javascript,${encodeURIComponent(loader)}`,
        new URL("validate-ci-summary.mjs", import.meta.url).pathname
      ],
      {
        env: {
          ...process.env,
          CI_CODE_ONLY: "false",
          CI_NEEDS: JSON.stringify({ code_checks: { result: "skipped" } }),
          CI_EVENT_NAME: "pull_request_review",
          GITHUB_REPOSITORY: "vercel/turborepo",
          GH_TOKEN: "test-token",
          GITHUB_RUN_ID: "123",
          PR_NUMBER: "456",
          PR_HEAD_SHA: "head-sha",
          GITHUB_SHA: "review-sha"
        },
        encoding: "utf8"
      }
    );
    assert.equal(output.status, failed ? 1 : 0);
    if (failed) {
      assert.match(output.stderr, /::error::No reusable code CI run/);
      assert.doesNotMatch(output.stdout, /All required CI jobs passed/);
    } else {
      assert.match(
        output.stdout,
        /Reused code checks from run 789\./
      );
      assert.match(output.stdout, /All required CI jobs passed/);
    }
  }
});

test("CLI exits nonzero for malformed dependency JSON", () => {
  const output = spawnSync(
    process.execPath,
    [new URL("validate-ci-summary.mjs", import.meta.url).pathname],
    {
      env: {
        ...process.env,
        CI_NEEDS: "{",
        CI_EVENT_NAME: "pull_request",
        CI_CODE_ONLY: "false"
      },
      encoding: "utf8"
    }
  );
  assert.equal(output.status, 1);
  assert.match(output.stderr, /::error::/);
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
          CI_EVENT_NAME: "pull_request",
          CI_CODE_ONLY: "true"
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
