#!/usr/bin/env node

import { execFileSync } from "node:child_process";
import { appendFileSync, lstatSync, readFileSync } from "node:fs";
import { pathToFileURL } from "node:url";

const MAX_FILES = Number(process.env.CREATE_GITHUB_API_COMMIT_MAX_FILES ?? 500);
const MAX_FILE_BYTES = Number(
  process.env.CREATE_GITHUB_API_COMMIT_MAX_FILE_BYTES ?? 2 * 1024 * 1024,
);
const MAX_TOTAL_BYTES = Number(
  process.env.CREATE_GITHUB_API_COMMIT_MAX_TOTAL_BYTES ?? 10 * 1024 * 1024,
);
const FETCH_TIMEOUT_MS = Number(
  process.env.CREATE_GITHUB_API_COMMIT_FETCH_TIMEOUT_MS ?? 30_000,
);

const SENSITIVE_PATH_PATTERNS = [
  /^\.env($|\.)/,
  /^\.npmrc$/,
  /(^|\/)(credentials|secrets?)\.(json|ya?ml|txt)$/i,
  /(^|\/)id_rsa$/,
];

function usage() {
  return `Usage: create-github-api-commit.mjs --branch <branch> --message <message> (--all-tracked | --path <path>...)

Creates a GitHub-verified commit on a branch using the GitHub API.

Options:
  --branch <branch>          Target branch name. Created from local HEAD if missing.
  --message <message>        Commit headline.
  --all-tracked              Include all tracked file changes compared to HEAD.
  --include-untracked        Include untracked files matched by the path selection.
  --path <path>              Include changes for one repository-relative path.
  --if-exists fail|update    Existing branch policy. Defaults to fail.
  --help                     Show this help text.

Environment:
  GH_TOKEN or GITHUB_TOKEN   GitHub token with contents:write.
  GITHUB_REPOSITORY          Repository in owner/name format.`;
}

export function parseArgs(args = process.argv.slice(2)) {
  const options = {
    allTracked: false,
    ifExists: "fail",
    includeUntracked: false,
    paths: [],
  };

  for (let i = 0; i < args.length; i += 1) {
    const arg = args[i];
    if (arg === "--branch" || arg === "--message") {
      const value = args[i + 1];
      if (!value) {
        throw new Error(`Missing value for ${arg}`);
      }
      options[arg.slice(2)] = value;
      i += 1;
    } else if (arg === "--path") {
      const value = args[i + 1];
      if (!value) {
        throw new Error("Missing value for --path");
      }
      options.paths.push(value);
      i += 1;
    } else if (arg === "--if-exists") {
      const value = args[i + 1];
      if (value !== "fail" && value !== "update") {
        throw new Error("--if-exists must be 'fail' or 'update'");
      }
      options.ifExists = value;
      i += 1;
    } else if (arg === "--all-tracked") {
      options.allTracked = true;
    } else if (arg === "--include-untracked") {
      options.includeUntracked = true;
    } else if (arg === "--help") {
      options.help = true;
    } else {
      throw new Error(`Unknown argument: ${arg}`);
    }
  }

  if (options.help) {
    return options;
  }
  if (!options.branch) {
    throw new Error("Missing required --branch argument");
  }
  if (!options.message) {
    throw new Error("Missing required --message argument");
  }
  if (!options.allTracked && options.paths.length === 0) {
    throw new Error("Specify --all-tracked or at least one --path");
  }
  if (options.allTracked && options.paths.length > 0) {
    throw new Error("Use either --all-tracked or --path, not both");
  }
  assertBranchName(options.branch);
  for (const path of options.paths) {
    assertRepoRelativePath(path);
  }

  return options;
}

function git(args, { trim = true } = {}) {
  const output = execFileSync("git", args, { encoding: "utf8" });
  return trim ? output.trim() : output;
}

function splitNull(output) {
  return output.split("\0").filter(Boolean);
}

function assertRepositoryName(value) {
  if (!/^[A-Za-z0-9_.-]+\/[A-Za-z0-9_.-]+$/.test(value)) {
    throw new Error(`Invalid GITHUB_REPOSITORY: ${value}`);
  }
}

function assertBranchName(branch) {
  git(["check-ref-format", "--branch", branch]);
  if (branch.startsWith("refs/")) {
    throw new Error(`Invalid branch name: ${branch}`);
  }
}

function assertRepoRelativePath(path) {
  if (
    path.length === 0 ||
    path.startsWith("/") ||
    path.includes("..") ||
    path.startsWith(":")
  ) {
    throw new Error(`Invalid commit path: ${path}`);
  }
}

function assertSafeCommitPath(path) {
  assertRepoRelativePath(path);
  if (SENSITIVE_PATH_PATTERNS.some((pattern) => pattern.test(path))) {
    throw new Error(`Refusing to commit sensitive-looking file: ${path}`);
  }
}

function getChangedPaths({ allTracked, includeUntracked, paths }) {
  const pathspec = allTracked ? [] : paths;
  const entries = [];
  const diff = splitNull(
    git(
      ["diff", "--name-status", "--no-renames", "-z", "HEAD", "--", ...pathspec],
      { trim: false },
    ),
  );

  for (let i = 0; i < diff.length; i += 2) {
    entries.push({ path: diff[i + 1], deleted: diff[i] === "D" });
  }

  if (includeUntracked) {
    const untracked = splitNull(
      git(
        ["ls-files", "--others", "--exclude-standard", "-z", "--", ...pathspec],
        { trim: false },
      ),
    );
    for (const path of untracked) {
      entries.push({ path, deleted: false });
    }
  }

  return entries;
}

function fileContents(path) {
  const stat = lstatSync(path);
  if (!stat.isFile()) {
    throw new Error(`${path} is not a regular file`);
  }
  if (stat.size > MAX_FILE_BYTES) {
    throw new Error(
      `${path} is ${stat.size} bytes, exceeding limit ${MAX_FILE_BYTES}`,
    );
  }
  return { contents: readFileSync(path).toString("base64"), size: stat.size };
}

function repository() {
  const value = process.env.GITHUB_REPOSITORY;
  if (!value) {
    throw new Error("Missing GITHUB_REPOSITORY");
  }
  assertRepositoryName(value);
  return value;
}

function token() {
  const value = process.env.GH_TOKEN ?? process.env.GITHUB_TOKEN;
  if (!value) {
    throw new Error("Missing GH_TOKEN or GITHUB_TOKEN");
  }
  return value;
}

async function fetchJson(url, options, retryable = false) {
  let lastError;

  for (let attempt = 1; attempt <= (retryable ? 3 : 1); attempt += 1) {
    const signal = AbortSignal.timeout(FETCH_TIMEOUT_MS);
    try {
      const response = await fetch(url, { ...options, signal });
      const data = await response.json();
      if (
        retryable &&
        (response.status === 429 || response.status >= 500) &&
        attempt < 3
      ) {
        await new Promise((resolve) => setTimeout(resolve, attempt * 1000));
        continue;
      }
      return { data, response };
    } catch (error) {
      lastError = error;
      if (!retryable || attempt === 3) {
        throw error;
      }
      await new Promise((resolve) => setTimeout(resolve, attempt * 1000));
    }
  }

  throw lastError;
}

async function githubRest(path) {
  const { data, response } = await fetchJson(`https://api.github.com${path}`, {
    headers: {
      Accept: "application/vnd.github+json",
      Authorization: `Bearer ${token()}`,
      "X-GitHub-Api-Version": "2022-11-28",
    },
  }, true);

  if (!response.ok) {
    throw new Error(
      `GET ${path} failed: ${data.message ?? response.statusText}`,
    );
  }

  return data;
}

async function githubGraphql(query, variables, { retryable = false } = {}) {
  const { data, response } = await fetchJson("https://api.github.com/graphql", {
    method: "POST",
    headers: {
      Accept: "application/vnd.github+json",
      Authorization: `Bearer ${token()}`,
      "Content-Type": "application/json",
    },
    body: JSON.stringify({ query, variables }),
  }, retryable);

  if (!response.ok || data.errors) {
    const message =
      data.errors?.map((error) => error.message).join("; ") ?? data.message;
    throw new Error(`GraphQL request failed: ${message}`);
  }

  return data.data;
}

async function getRepositoryInfo(repositoryName, branch) {
  const [owner, repo] = repositoryName.split("/");
  const data = await githubGraphql(
    `query($owner: String!, $repo: String!, $ref: String!) {
      repository(owner: $owner, name: $repo) {
        id
        nameWithOwner
        ref(qualifiedName: $ref) {
          target {
            ... on Commit {
              oid
            }
          }
        }
      }
    }`,
    { owner, repo, ref: `refs/heads/${branch}` },
    { retryable: true },
  );

  return data.repository;
}

async function createBranch(repositoryId, branch, oid) {
  const data = await githubGraphql(
    `mutation($input: CreateRefInput!) {
      createRef(input: $input) {
        ref {
          id
        }
      }
    }`,
    {
      input: {
        repositoryId,
        name: `refs/heads/${branch}`,
        oid,
      },
    },
  );

  return data.createRef.ref.id;
}

export function buildFileChanges(changes) {
  const additions = [];
  const deletions = [];
  const seen = new Set();
  let totalBytes = 0;

  if (changes.length > MAX_FILES) {
    throw new Error(
      `Refusing to commit ${changes.length} files, exceeding limit ${MAX_FILES}`,
    );
  }

  for (const change of changes) {
    assertSafeCommitPath(change.path);
    if (seen.has(change.path)) {
      continue;
    }
    seen.add(change.path);

    if (change.deleted) {
      deletions.push({ path: change.path });
    } else {
      const file = fileContents(change.path);
      totalBytes += file.size;
      if (totalBytes > MAX_TOTAL_BYTES) {
        throw new Error(
          `Refusing to commit ${totalBytes} bytes, exceeding limit ${MAX_TOTAL_BYTES}`,
        );
      }
      additions.push({
        path: change.path,
        contents: file.contents,
      });
    }
  }

  const fileChanges = {};
  if (additions.length > 0) {
    fileChanges.additions = additions;
  }
  if (deletions.length > 0) {
    fileChanges.deletions = deletions;
  }

  return fileChanges;
}

async function createCommit({
  branch,
  expectedHeadOid,
  fileChanges,
  message,
  repositoryName,
}) {
  const data = await githubGraphql(
    `mutation($input: CreateCommitOnBranchInput!) {
      createCommitOnBranch(input: $input) {
        commit {
          oid
        }
      }
    }`,
    {
      input: {
        branch: { repositoryNameWithOwner: repositoryName, branchName: branch },
        expectedHeadOid,
        message: { headline: message },
        fileChanges,
      },
    },
  );

  return data.createCommitOnBranch.commit.oid;
}

async function assertVerified(repositoryName, sha) {
  let reason = "unknown";
  const attempts = Number(
    process.env.CREATE_GITHUB_API_COMMIT_VERIFY_ATTEMPTS ?? 5,
  );
  const delayMs = Number(process.env.CREATE_GITHUB_API_COMMIT_VERIFY_DELAY_MS);

  for (let attempt = 1; attempt <= attempts; attempt += 1) {
    try {
      const commit = await githubRest(`/repos/${repositoryName}/commits/${sha}`);
      if (commit.commit.verification.verified) {
        return;
      }
      reason = commit.commit.verification.reason;
    } catch (error) {
      reason = error instanceof Error ? error.message : String(error);
    }

    await new Promise((resolve) =>
      setTimeout(resolve, Number.isNaN(delayMs) ? attempt * 1000 : delayMs),
    );
  }

  throw new Error(`GitHub did not verify commit ${sha}: ${reason}`);
}

export async function run(options = parseArgs()) {
  if (options.help) {
    console.log(usage());
    return;
  }

  process.chdir(git(["rev-parse", "--show-toplevel"]));

  const repositoryName = repository();
  const currentHead = git(["rev-parse", "HEAD"]);
  const currentBranch = git(["branch", "--show-current"]);
  const changes = getChangedPaths(options);
  if (changes.length === 0) {
    throw new Error("No changes to commit");
  }
  const fileChanges = buildFileChanges(changes);

  const repositoryInfo = await getRepositoryInfo(repositoryName, options.branch);
  let expectedHeadOid = repositoryInfo.ref?.target?.oid;
  if (expectedHeadOid && options.ifExists === "fail") {
    throw new Error(
      `Remote branch ${options.branch} already exists at ${expectedHeadOid}. Use --if-exists update to commit onto the remote branch.`,
    );
  }

  if (!expectedHeadOid) {
    await createBranch(repositoryInfo.id, options.branch, currentHead);
    expectedHeadOid = currentHead;
  }

  const commitSha = await createCommit({
    branch: options.branch,
    expectedHeadOid,
    fileChanges,
    message: options.message,
    repositoryName,
  });

  try {
    await assertVerified(repositoryName, commitSha);
  } catch (error) {
    console.error(
      `Created ${commitSha} on ${options.branch}, but verification did not complete. Leaving the branch in place for recovery.`,
    );
    throw error;
  }

  git(["update-ref", `refs/heads/${options.branch}`, commitSha]);
  if (currentBranch === options.branch) {
    git(["reset", "--mixed", commitSha]);
  }

  console.log(`Created ${commitSha} on ${options.branch}`);

  if (process.env.GITHUB_OUTPUT) {
    appendFileSync(process.env.GITHUB_OUTPUT, `commit-sha=${commitSha}\n`);
  }
}

if (import.meta.url === pathToFileURL(process.argv[1]).href) {
  run().catch((error) => {
    console.error(error.message);
    process.exit(1);
  });
}
