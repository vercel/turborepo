#!/usr/bin/env node

import {
  CLI_PACKAGE_PATHS,
  LIBRARY_PACKAGE_PATHS,
  run as validateRelease,
} from "./validate-release-pr.mjs";

const WRITE_PERMISSIONS = new Set(["admin", "maintain", "write"]);

function requiredEnv(name) {
  const value = process.env[name];
  if (!value) {
    throw new Error(`Missing ${name}`);
  }
  return value;
}

async function github(path) {
  const response = await fetch(`https://api.github.com${path}`, {
    headers: {
      Accept: "application/vnd.github+json",
      Authorization: `Bearer ${requiredEnv("GH_TOKEN")}`,
      "X-GitHub-Api-Version": "2022-11-28",
    },
  });
  if (!response.ok) {
    throw new Error(`GitHub API request failed (${response.status}): ${path}`);
  }
  return response.json();
}

function isReleaseReviewPath(path) {
  return (
    path === "version.txt" ||
    CLI_PACKAGE_PATHS.includes(path) ||
    LIBRARY_PACKAGE_PATHS.includes(path) ||
    /^skills\/turborepo\/.*\.md$/.test(path)
  );
}

async function hasNonReleasePath(repository, pullNumber) {
  for (let page = 1; ; page += 1) {
    const files = await github(
      `/repos/${repository}/pulls/${pullNumber}/files?per_page=100&page=${page}`,
    );
    if (files.some(({ filename }) => !isReleaseReviewPath(filename))) {
      return true;
    }
    if (files.length < 100) {
      return false;
    }
  }
}

async function hasValidApproval(repository, pull) {
  const reviews = [];
  for (let page = 1; ; page += 1) {
    const batch = await github(
      `/repos/${repository}/pulls/${pull.number}/reviews?per_page=100&page=${page}`,
    );
    reviews.push(...batch);
    if (batch.length < 100) {
      break;
    }
  }
  const latestByUser = new Map();
  for (const review of reviews) {
    if (review.user?.login && review.state !== "COMMENTED") {
      latestByUser.set(review.user.login, review);
    }
  }

  let approved = false;
  for (const [login, review] of latestByUser) {
    if (login === pull.user.login || review.user.type !== "User") {
      continue;
    }
    const permission = await github(
      `/repos/${repository}/collaborators/${encodeURIComponent(login)}/permission`,
    );
    if (!WRITE_PERMISSIONS.has(permission.permission)) {
      continue;
    }
    if (review.state === "CHANGES_REQUESTED") {
      throw new Error(`Changes are requested by ${login}`);
    }
    approved ||=
      review.state === "APPROVED" && review.commit_id === pull.head.sha;
  }
  return approved;
}

function setReleaseEnvironment(pull) {
  process.env.PR_BASE_SHA = pull.base.sha;
  process.env.PR_HEAD_SHA = pull.head.sha;
  process.env.PR_HEAD_REF = pull.head.ref;
  process.env.PR_TITLE = pull.title;
}

export async function run() {
  const repository = requiredEnv("GITHUB_REPOSITORY");
  const pullNumber = requiredEnv("PR_NUMBER");
  const pull = await github(`/repos/${repository}/pulls/${pullNumber}`);

  if (
    pull.base.ref !== pull.base.repo.default_branch ||
    pull.base.repo.full_name !== repository ||
    pull.state !== "open" ||
    pull.draft
  ) {
    throw new Error("Pull request metadata is not eligible for review");
  }

  const expectedHeadSha = process.env.EXPECTED_HEAD_SHA;
  if (expectedHeadSha && pull.head.sha !== expectedHeadSha) {
    throw new Error("Pull request head does not match the dispatched SHA");
  }
  if (expectedHeadSha && process.env.GITHUB_SHA !== expectedHeadSha) {
    throw new Error("Review gate is not running on the dispatched SHA");
  }

  if (await hasNonReleasePath(repository, pull.number)) {
    console.log("Native team review policy applies to this pull request");
    return;
  }

  const isAutomatedRelease =
    pull.user.login === "github-actions[bot]" &&
    pull.user.id === 41898282 &&
    pull.head.repo.full_name === repository &&
    (/^staging-/.test(pull.head.ref) ||
      /^library-release\//.test(pull.head.ref));
  if (isAutomatedRelease) {
    setReleaseEnvironment(pull);
    await validateRelease();
    return;
  }

  if (!(await hasValidApproval(repository, pull))) {
    throw new Error(
      "A write-authorized human must approve non-release changes to review-exempt files",
    );
  }
}

if (import.meta.url === `file://${process.argv[1]}`) {
  run().catch((error) => {
    console.error(`::error::${error.message}`);
    process.exitCode = 1;
  });
}
