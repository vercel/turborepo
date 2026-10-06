#!/usr/bin/env node

export const JOBS = [
  "release-pr",
  "quality",
  "js_tests",
  "review_gate",
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
const PR_ONLY_JOBS = new Set(["quality", "js_tests", "review_gate"]);

export function validateSummary(needs, eventName) {
  if (!["pull_request", "pull_request_review", "push"].includes(eventName)) {
    throw new Error(`Unexpected CI event: ${eventName}`);
  }
  if (!needs || typeof needs !== "object" || Array.isArray(needs)) {
    throw new Error("Missing CI dependency results");
  }
  for (const job of Object.keys(needs)) {
    if (!JOBS.includes(job)) {
      throw new Error(`Unexpected CI dependency: ${job}`);
    }
  }

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

if (import.meta.url === `file://${process.argv[1]}`) {
  try {
    validateSummary(
      JSON.parse(process.env.CI_NEEDS ?? "null"),
      process.env.CI_EVENT_NAME
    );
    console.log("All required CI jobs passed or were intentionally skipped.");
  } catch (error) {
    console.error(`::error::${error.message}`);
    process.exitCode = 1;
  }
}
