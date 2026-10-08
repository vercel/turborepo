#!/usr/bin/env node

import { setTimeout } from "node:timers/promises";

const WORKFLOW_PATH = ".github/workflows/turborepo-test.yml";
const POLL_INTERVAL_MS = 15_000;
const TIMEOUT_MS = 30 * 60 * 1000;
const ACTIVE_STATUSES = ["queued", "in_progress", "waiting", "pending", "requested"];
const positiveInteger = (value) => Number.isSafeInteger(value) && value > 0;
const nonemptyString = (value) => typeof value === "string" && value.length > 0;

// Reuse only code results; the caller must evaluate the review policy afresh.
// request(path) returns parsed GitHub API JSON, and wait(delay) is injectable.
export async function validateReviewCI({
  repository,
  token,
  runId,
  pullNumber,
  headSha,
  reviewSha = headSha,
  request,
  wait = setTimeout,
  maxAttempts = 120,
} = {}) {
  if (
    !/^[A-Za-z0-9_.-]+\/[A-Za-z0-9_.-]+$/.test(repository ?? "") ||
    !nonemptyString(token) ||
    !positiveInteger(runId) ||
    !positiveInteger(pullNumber) ||
    !nonemptyString(headSha) ||
    !nonemptyString(reviewSha) ||
    !positiveInteger(maxAttempts) ||
    maxAttempts > 120 ||
    (request !== undefined && typeof request !== "function") ||
    typeof wait !== "function"
  ) {
    throw new Error("Invalid review CI configuration");
  }

  const deadline = Date.now() + TIMEOUT_MS;
  function remaining() {
    const time = deadline - Date.now();
    if (time <= 0) throw new Error("Timed out waiting for code checks");
    return time;
  }
  request ??= async (path) => {
    // Never follow payload URLs or redirects, and never use a write API.
    const response = await fetch(`https://api.github.com${path}`, {
      method: "GET",
      redirect: "error",
      signal: AbortSignal.timeout(Math.min(30_000, remaining())),
      headers: {
        Accept: "application/vnd.github+json",
        Authorization: `Bearer ${token}`,
        "X-GitHub-Api-Version": "2022-11-28",
      },
    });
    if (!response.ok) {
      throw new Error(`GitHub API request failed (${response.status}): ${path}`);
    }
    return response.json();
  };
  async function github(path) {
    remaining();
    const data = await request(path);
    remaining();
    return data;
  }

  const root = `/repos/${repository}`;
  const currentRun = await github(`${root}/actions/runs/${runId}`);
  if (
    currentRun?.id !== runId ||
    currentRun.event !== "pull_request_review" ||
    currentRun.path !== WORKFLOW_PATH ||
    !positiveInteger(currentRun.workflow_id) ||
    !positiveInteger(currentRun.repository?.id) ||
    currentRun.repository.full_name !== repository ||
    (currentRun.head_sha !== headSha && currentRun.head_sha !== reviewSha)
  ) {
    throw new Error("Current review run has unexpected workflow, head, or repository");
  }

  const repoId = currentRun.repository.id;
  let expected;
  const matchesRepository = (repo) =>
    repo?.id === repoId && repo?.full_name === repository;
  async function checkPull() {
    const pull = await github(`${root}/pulls/${pullNumber}`);
    if (
      pull?.number !== pullNumber ||
      pull.state !== "open" ||
      pull.draft !== false ||
      pull.head?.sha !== headSha ||
      !positiveInteger(pull.head?.repo?.id) ||
      !nonemptyString(pull.head.repo.full_name) ||
      !nonemptyString(pull.head.ref) ||
      !matchesRepository(pull.base?.repo) ||
      !nonemptyString(pull.base?.ref) ||
      pull.base.ref !== pull.base.repo.default_branch ||
      (expected &&
        (pull.head.repo.id !== expected.head.repo.id ||
          pull.head.repo.full_name !== expected.head.repo.full_name ||
          pull.head.ref !== expected.head.ref ||
          pull.base.ref !== expected.base.ref))
    ) {
      throw new Error("Pull request revision changed or is not eligible for review");
    }
    expected ??= pull;
  }

  const matchesHead = (run, event = "pull_request") =>
    (run?.head_sha === headSha ||
      (event === "pull_request_review" && run?.head_sha === reviewSha)) &&
    run.head_repository?.id === expected.head.repo.id &&
    run.head_repository?.full_name === expected.head.repo.full_name &&
    run.head_branch === expected.head.ref;

  function checkRun(run, event) {
    const pulls = run?.pull_requests;
    const associations = Array.isArray(pulls)
      ? pulls.filter((pull) => pull?.number === pullNumber)
      : [];
    const association = associations[0];
    if (
      !positiveInteger(run?.id) ||
      !positiveInteger(run.run_number) ||
      !positiveInteger(run.run_attempt) ||
      run.workflow_id !== currentRun.workflow_id ||
      run.path !== WORKFLOW_PATH ||
      run.event !== event ||
      !matchesHead(run, event) ||
      !matchesRepository(run.repository) ||
      !Array.isArray(pulls) ||
      pulls.some((pull) => !positiveInteger(pull?.number)) ||
      (pulls.length > 0 &&
        (associations.length !== 1 ||
          association.head?.sha !== headSha ||
          association.head?.ref !== expected.head.ref ||
          association.head?.repo?.id !== expected.head.repo.id ||
          association.base?.ref !== expected.base.ref ||
          association.base?.repo?.id !== repoId))
    ) {
      throw new Error("CI run does not match this workflow and PR revision");
    }
  }

  async function latestRun() {
    let latest;
    for (let page = 1; ; page += 1) {
      const query = new URLSearchParams({
        event: "pull_request",
        head_sha: headSha,
        per_page: "100",
        page: String(page),
      });
      const data = await github(
        `${root}/actions/workflows/${currentRun.workflow_id}/runs?${query}`,
      );
      if (!Array.isArray(data?.workflow_runs)) {
        throw new Error("Missing workflow run results");
      }
      for (const run of data.workflow_runs) {
        if (
          !Array.isArray(run?.pull_requests) ||
          run.pull_requests.some((pull) => !positiveInteger(pull?.number))
        ) {
          throw new Error("Cannot prove workflow run PR association");
        }
        // GitHub can omit associations for fork PRs. Such candidates must
        // match the live PR's exact head; the aggregate then binds PR + base.
        if (
          run.pull_requests.length === 0
            ? !matchesHead(run)
            : !run.pull_requests.some((pull) => pull.number === pullNumber)
        ) {
          continue;
        }
        // Select candidates before checking remaining identity or results:
        // never fall back from a newer failed or mismatched candidate.
        if (!positiveInteger(run.run_number) || !positiveInteger(run.id)) {
          throw new Error("Invalid workflow run identity");
        }
        if (
          !latest ||
          run.run_number > latest.run_number ||
          (run.run_number === latest.run_number && run.id > latest.id)
        ) {
          latest = run;
        }
      }
      if (data.workflow_runs.length < 100) break;
      // GitHub caps filtered workflow run searches at 1,000 results.
      if (page === 10) throw new Error("Workflow run search exceeds API limit");
    }
    // The exact revision's run may not be visible yet; let the caller poll.
    if (!latest) return undefined;
    checkRun(latest, "pull_request");
    const detail = await github(`${root}/actions/runs/${latest.id}`);
    checkRun(detail, "pull_request");
    if (detail.id !== latest.id || detail.run_number !== latest.run_number) {
      throw new Error("Unexpected code CI run identity");
    }
    return detail;
  }

  async function checkJobs(run) {
    const aggregates = [];
    const aggregateName = `Code checks (PR #${pullNumber}, ${expected.base.ref})`;
    for (let page = 1; ; page += 1) {
      // Pin the attempt so reruns cannot reuse an older attempt's success.
      const data = await github(
        `${root}/actions/runs/${run.id}/attempts/${run.run_attempt}/jobs?per_page=100&page=${page}`,
      );
      if (!Array.isArray(data?.jobs)) throw new Error("Missing code CI job results");
      // The original event's PR/base label is authoritative even when the
      // run's pull_requests association is empty. Never accept a plain label.
      aggregates.push(...data.jobs.filter((job) => job?.name === aggregateName));
      if (data.jobs.length < 100) break;
      if (page === 100) throw new Error("Code CI job search exceeds safety limit");
    }
    if (
      aggregates.length > 1 ||
      (aggregates.length === 1 &&
        aggregates[0].status === "completed" &&
        aggregates[0].conclusion !== "success")
    ) {
      throw new Error("Code checks must have exactly one completed, successful aggregate");
    }
    if (aggregates.length === 1 && aggregates[0].status === "completed") {
      return true;
    }
    if (run.status === "completed") {
      throw new Error("Code checks must have exactly one completed, successful aggregate");
    }
    // Active runs can still be creating or executing the aggregate.
    return false;
  }

  await checkPull();
  checkRun(currentRun, "pull_request_review");
  for (let attempt = 0; attempt < maxAttempts; attempt += 1) {
    if (attempt > 0) await checkPull();
    const run = await latestRun();
    if (run) {
      if (run.status === "completed") {
        // The overall run may have failed only because its review policy failed.
        if (!["success", "failure"].includes(run.conclusion)) {
          throw new Error(`Code CI run has unacceptable conclusion: ${run.conclusion}`);
        }
      } else if (!ACTIVE_STATUSES.includes(run.status)) {
        throw new Error(`Unexpected code CI run status: ${run.status}`);
      }
      if (await checkJobs(run)) {
        const finalRun = await latestRun();
        if (
          !finalRun ||
          finalRun.id !== run.id ||
          finalRun.run_attempt !== run.run_attempt ||
          (finalRun.status === "completed"
            ? !["success", "failure"].includes(finalRun.conclusion)
            : !ACTIVE_STATUSES.includes(finalRun.status))
        ) {
          throw new Error("Latest code CI run or attempt changed during validation");
        }
        await checkPull();
        remaining();
        return run.id;
      }
    }
    if (attempt + 1 === maxAttempts) break;
    await wait(Math.min(POLL_INTERVAL_MS, remaining()));
  }
  throw new Error("Timed out waiting for code checks");
}
