#!/usr/bin/env node

import { validateReviewCI } from "./validate-review-ci.mjs";

export const JOBS = [
  "release-pr",
  "quality",
  "js_tests",
  "turbo_types_check",
  "rust_test",
  "check-examples",
  "check-lockfiles",
  "js_native_packages"
];

const RELEASE_TEST_JOBS = new Set([
  "js_tests",
  "turbo_types_check",
  "rust_test",
  "check-examples",
  "check-lockfiles",
  "js_native_packages"
]);
const PR_ONLY_JOBS = new Set(["quality", "js_tests"]);

function validateDependencies(needs, jobs) {
  if (!needs || typeof needs !== "object" || Array.isArray(needs)) {
    throw new Error("Missing CI dependency results");
  }
  for (const job of Object.keys(needs)) {
    if (!jobs.includes(job)) {
      throw new Error(`Unexpected CI dependency: ${job}`);
    }
  }
}

export function validateSummary(needs, eventName) {
  if (!["pull_request", "push"].includes(eventName)) {
    throw new Error(`Unexpected CI event: ${eventName}`);
  }
  validateDependencies(needs, JOBS);

  const release = needs["release-pr"]?.outputs?.["is-release-pr"];
  if (
    !["true", "false"].includes(release) ||
    (eventName === "push" && release !== "false")
  ) {
    throw new Error("Missing or invalid release PR validation output");
  }

  const failures = [];
  for (const job of JOBS) {
    const result = needs[job]?.result;
    const allowedSkip =
      (release === "true" && RELEASE_TEST_JOBS.has(job)) ||
      (eventName === "push" && PR_ONLY_JOBS.has(job)) ||
      (eventName !== "push" && job === "check-examples");
    if (result !== "success" && !(result === "skipped" && allowedSkip)) {
      failures.push(`${job}: ${result ?? "missing"}`);
    }
  }
  if (failures.length > 0) {
    throw new Error(`CI did not pass:\n${failures.join("\n")}`);
  }
}

export function validateFinalSummary(needs, eventName) {
  if (!["pull_request", "pull_request_review", "push"].includes(eventName)) {
    throw new Error(`Unexpected CI event: ${eventName}`);
  }
  validateDependencies(needs, ["code_checks"]);
  const expected = eventName === "pull_request_review" ? "skipped" : "success";
  const result = needs.code_checks?.result;
  if (result !== expected) {
    throw new Error(`CI did not pass:\ncode_checks: ${result ?? "missing"}`);
  }
}

if (import.meta.url === `file://${process.argv[1]}`) {
  try {
    const needs = JSON.parse(process.env.CI_NEEDS ?? "null");
    const eventName = process.env.CI_EVENT_NAME;
    if (process.env.CI_CODE_ONLY === "true") {
      validateSummary(needs, eventName);
    } else {
      validateFinalSummary(needs, eventName);
      if (eventName === "pull_request_review") {
        const runId = await validateReviewCI({
          repository: process.env.GITHUB_REPOSITORY,
          token: process.env.GH_TOKEN,
          runId: Number(process.env.GITHUB_RUN_ID),
          pullNumber: Number(process.env.PR_NUMBER),
          headSha: process.env.PR_HEAD_SHA,
          reviewSha: process.env.GITHUB_SHA
        });
        console.log(`Reused code checks from run ${runId}.`);
      }
    }
    console.log("All required CI jobs passed or were intentionally skipped.");
  } catch (error) {
    console.error(`::error::${error.message}`);
    process.exitCode = 1;
  }
}
