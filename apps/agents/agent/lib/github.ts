import { createSign } from "node:crypto";

export interface RepositoryReference {
  owner: string;
  repo: string;
}

export interface PullRequestSummary {
  number: number;
  title: string;
  url: string;
  headRef: string;
  baseRef: string;
  updatedAt: string | null;
}

export interface CommitSummary {
  sha: string;
  committedAt: string | null;
}

interface InstallationTokenResponse {
  expires_at?: string;
  token?: string;
}

interface PullRequestListEntry {
  number?: number;
  title?: string;
  html_url?: string;
  updated_at?: string;
  head?: { ref?: string };
  base?: { ref?: string };
}

interface PullRequestFileEntry {
  filename?: string;
  previous_filename?: string;
}

interface CommitListEntry {
  sha?: string;
  commit?: {
    author?: { date?: string };
    committer?: { date?: string };
  };
}

const FILES_PER_PAGE = 100;
const PULL_REQUESTS_PER_PAGE = 100;

let cachedInstallationToken:
  | { expiresAt: number; installationId: number; token: string }
  | undefined;

export class GitHubApiError extends Error {
  constructor(
    message: string,
    readonly status: number
  ) {
    super(message);
  }
}

export function resolveRepository(input: {
  owner?: string;
  repo?: string;
}): RepositoryReference {
  if (input.owner && input.repo) {
    return { owner: input.owner, repo: input.repo };
  }

  const [owner, repo] = (process.env.GITHUB_REPOSITORY ?? "").split("/");
  if (owner && repo) {
    return { owner: input.owner ?? owner, repo: input.repo ?? repo };
  }

  throw new Error(
    "Pass owner and repo, or set GITHUB_REPOSITORY to 'owner/repo'."
  );
}

export async function githubRequest<T>(input: {
  body?: unknown;
  method: "GET" | "PATCH" | "POST";
  owner: string;
  path: string;
  repo: string;
}): Promise<T> {
  const response = await fetch(
    `https://api.github.com${repoPath(input.owner, input.repo, input.path)}`,
    {
      method: input.method,
      headers: {
        accept: "application/vnd.github+json",
        authorization: `Bearer ${await getInstallationToken()}`,
        "content-type": "application/json",
        "x-github-api-version": "2022-11-28"
      },
      body: input.body === undefined ? undefined : JSON.stringify(input.body)
    }
  );

  if (!response.ok) {
    throw new GitHubApiError(
      `GitHub ${input.method} ${input.path} failed with ${response.status}: ${await response.text()}`,
      response.status
    );
  }

  return (await response.json()) as T;
}

export async function listOpenPullRequests(input: {
  owner: string;
  repo: string;
  limit: number;
}): Promise<PullRequestSummary[]> {
  const pullRequests: PullRequestSummary[] = [];

  for (let page = 1; pullRequests.length < input.limit; page += 1) {
    const query = new URLSearchParams({
      state: "open",
      sort: "updated",
      direction: "desc",
      per_page: String(PULL_REQUESTS_PER_PAGE),
      page: String(page)
    });
    const batch = await githubRequest<PullRequestListEntry[]>({
      method: "GET",
      owner: input.owner,
      repo: input.repo,
      path: `/pulls?${query.toString()}`
    });

    for (const entry of batch) {
      if (typeof entry.number !== "number") {
        continue;
      }
      pullRequests.push({
        number: entry.number,
        title: entry.title ?? "",
        url: entry.html_url ?? "",
        headRef: entry.head?.ref ?? "",
        baseRef: entry.base?.ref ?? "",
        updatedAt: entry.updated_at ?? null
      });
    }

    if (batch.length < PULL_REQUESTS_PER_PAGE) {
      break;
    }
  }

  return pullRequests.slice(0, input.limit);
}

export async function listPullRequestFiles(input: {
  owner: string;
  repo: string;
  pullNumber: number;
  maxFiles: number;
}): Promise<string[]> {
  const files: string[] = [];

  for (let page = 1; files.length < input.maxFiles; page += 1) {
    const query = new URLSearchParams({
      per_page: String(FILES_PER_PAGE),
      page: String(page)
    });
    const batch = await githubRequest<PullRequestFileEntry[]>({
      method: "GET",
      owner: input.owner,
      repo: input.repo,
      path: `/pulls/${input.pullNumber}/files?${query.toString()}`
    });

    for (const entry of batch) {
      if (entry.filename) {
        files.push(entry.filename);
      }
      if (entry.previous_filename) {
        files.push(entry.previous_filename);
      }
    }

    if (batch.length < FILES_PER_PAGE) {
      break;
    }
  }

  return files;
}

export async function getLastCommitForPath(input: {
  owner: string;
  repo: string;
  path: string;
  ref?: string;
}): Promise<CommitSummary | null> {
  const query = new URLSearchParams({ path: input.path, per_page: "1" });
  if (input.ref) {
    query.set("sha", input.ref);
  }

  const commits = await githubRequest<CommitListEntry[]>({
    method: "GET",
    owner: input.owner,
    repo: input.repo,
    path: `/commits?${query.toString()}`
  });

  const commit = commits[0];
  if (!commit?.sha) {
    return null;
  }

  return {
    sha: commit.sha,
    committedAt:
      commit.commit?.committer?.date ?? commit.commit?.author?.date ?? null
  };
}

function repoPath(owner: string, repo: string, path: string) {
  return `/repos/${encodeURIComponent(owner)}/${encodeURIComponent(repo)}${path}`;
}

function requireEnv(name: string) {
  const value = process.env[name];
  if (!value) throw new Error(`Set ${name} before calling the GitHub API.`);
  return value;
}

function installationId() {
  const value = process.env.GITHUB_INSTALLATION_ID;
  if (!value) {
    throw new Error(
      "Set GITHUB_INSTALLATION_ID before calling the GitHub API."
    );
  }

  const parsed = Number(value);
  if (!Number.isInteger(parsed)) {
    throw new TypeError("GITHUB_INSTALLATION_ID must be an integer.");
  }

  return parsed;
}

function base64Url(value: Buffer | string) {
  return Buffer.from(value)
    .toString("base64")
    .replace(/[=]/g, "")
    .replace(/\+/g, "-")
    .replace(/\//g, "_");
}

function createGitHubAppJwt() {
  const now = Math.floor(Date.now() / 1000);
  const privateKey = requireEnv("GITHUB_APP_PRIVATE_KEY").replace(/\\n/g, "\n");
  const payload = {
    iat: now - 60,
    exp: now + 9 * 60,
    iss: requireEnv("GITHUB_APP_ID")
  };
  const unsigned = `${base64Url(JSON.stringify({ alg: "RS256", typ: "JWT" }))}.${base64Url(JSON.stringify(payload))}`;
  const signature = createSign("RSA-SHA256").update(unsigned).sign(privateKey);

  return `${unsigned}.${base64Url(signature)}`;
}

async function getInstallationToken() {
  const id = installationId();
  if (
    cachedInstallationToken?.installationId === id &&
    cachedInstallationToken.expiresAt > Date.now() + 60_000
  ) {
    return cachedInstallationToken.token;
  }

  const response = await fetch(
    `https://api.github.com/app/installations/${id}/access_tokens`,
    {
      method: "POST",
      headers: {
        accept: "application/vnd.github+json",
        authorization: `Bearer ${createGitHubAppJwt()}`,
        "x-github-api-version": "2022-11-28"
      }
    }
  );

  if (!response.ok) {
    throw new Error(
      `GitHub token request failed with ${response.status}: ${await response.text()}`
    );
  }

  const body = (await response.json()) as InstallationTokenResponse;
  if (!body.token || !body.expires_at) {
    throw new Error(
      "GitHub token response did not include token and expires_at."
    );
  }

  cachedInstallationToken = {
    expiresAt: Date.parse(body.expires_at),
    installationId: id,
    token: body.token
  };

  return body.token;
}
