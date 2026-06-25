import { createSign } from "node:crypto";
import { defineTool } from "eve/tools";
import { always } from "eve/tools/approval";
import { z } from "zod";

const filePathSchema = z
  .string()
  .min(1)
  .refine((path) => !path.startsWith("/") && !path.includes(".."), {
    message: "Use a relative path without '..'."
  });

const inputSchema = z.object({
  owner: z.string().min(1),
  repo: z.string().min(1),
  baseBranch: z.string().min(1).default("main"),
  branchName: z
    .string()
    .regex(/^agents\/[A-Za-z0-9._\/-]+$/, "Branch must start with agents/"),
  title: z.string().min(1),
  body: z.string().default(""),
  commitMessage: z.string().min(1),
  draft: z.boolean().default(false),
  files: z
    .array(
      z.object({
        path: filePathSchema.describe(
          "Path to write in the GitHub repository."
        ),
        sandboxPath: filePathSchema
          .optional()
          .describe("Path to read from the sandbox. Defaults to path.")
      })
    )
    .min(1)
});

type RefResponse = { object?: { sha?: string } };
type CommitResponse = { tree?: { sha?: string } };
type ShaResponse = { sha?: string };
type InstallationTokenResponse = { expires_at?: string; token?: string };
type PullRequestResponse = { html_url?: string; number?: number };

let cachedInstallationToken:
  | { expiresAt: number; installationId: number; token: string }
  | undefined;

class GitHubApiError extends Error {
  constructor(
    message: string,
    readonly status: number
  ) {
    super(message);
  }
}

function installationId() {
  const value = process.env.GITHUB_INSTALLATION_ID;
  if (!value) {
    throw new Error(
      "Set GITHUB_INSTALLATION_ID before creating pull requests."
    );
  }

  const parsed = Number(value);
  if (!Number.isInteger(parsed)) {
    throw new Error("GITHUB_INSTALLATION_ID must be an integer.");
  }

  return parsed;
}

function repoPath(owner: string, repo: string, path: string) {
  return `/repos/${encodeURIComponent(owner)}/${encodeURIComponent(repo)}${path}`;
}

function requireSha(value: string | undefined, label: string) {
  if (!value) throw new Error(`GitHub response did not include ${label}.`);
  return value;
}

function requireEnv(name: string) {
  const value = process.env[name];
  if (!value) throw new Error(`Set ${name} before creating pull requests.`);
  return value;
}

function base64Url(value: Buffer | string) {
  return Buffer.from(value)
    .toString("base64")
    .replace(/=/g, "")
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

async function github<T>(input: {
  body?: unknown;
  method: "GET" | "PATCH" | "POST";
  owner: string;
  path: string;
  repo: string;
}) {
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

export default defineTool({
  description:
    "Create a GitHub pull request from selected sandbox files. Use only after editing files in the sandbox and choosing an agents/* branch name.",
  inputSchema,
  needsApproval: always(),
  async execute(input, ctx) {
    const sandbox = await ctx.getSandbox();
    const baseRef = await github<RefResponse>({
      method: "GET",
      owner: input.owner,
      repo: input.repo,
      path: `/git/ref/heads/${input.baseBranch}`
    });
    const baseCommitSha = requireSha(baseRef.object?.sha, "base ref SHA");

    let branchCommitSha = baseCommitSha;
    try {
      const branchRef = await github<RefResponse>({
        method: "GET",
        owner: input.owner,
        repo: input.repo,
        path: `/git/ref/heads/${input.branchName}`
      });
      branchCommitSha = requireSha(branchRef.object?.sha, "branch ref SHA");
    } catch (error) {
      if (!(error instanceof GitHubApiError) || error.status !== 404) {
        throw error;
      }

      await github({
        method: "POST",
        owner: input.owner,
        repo: input.repo,
        path: "/git/refs",
        body: {
          ref: `refs/heads/${input.branchName}`,
          sha: baseCommitSha
        }
      });
    }

    const branchCommit = await github<CommitResponse>({
      method: "GET",
      owner: input.owner,
      repo: input.repo,
      path: `/git/commits/${branchCommitSha}`
    });
    const baseTreeSha = requireSha(branchCommit.tree?.sha, "base tree SHA");

    const tree = await Promise.all(
      input.files.map(async (file) => {
        const content = await sandbox.readTextFile({
          path: file.sandboxPath ?? file.path
        });
        const blob = await github<ShaResponse>({
          method: "POST",
          owner: input.owner,
          repo: input.repo,
          path: "/git/blobs",
          body: { content, encoding: "utf-8" }
        });

        return {
          path: file.path,
          mode: "100644",
          type: "blob",
          sha: requireSha(blob.sha, "blob SHA")
        };
      })
    );

    const newTree = await github<ShaResponse>({
      method: "POST",
      owner: input.owner,
      repo: input.repo,
      path: "/git/trees",
      body: {
        base_tree: baseTreeSha,
        tree
      }
    });

    const commit = await github<ShaResponse>({
      method: "POST",
      owner: input.owner,
      repo: input.repo,
      path: "/git/commits",
      body: {
        message: input.commitMessage,
        tree: requireSha(newTree.sha, "tree SHA"),
        parents: [branchCommitSha]
      }
    });
    const newCommitSha = requireSha(commit.sha, "commit SHA");

    await github({
      method: "PATCH",
      owner: input.owner,
      repo: input.repo,
      path: `/git/refs/heads/${input.branchName}`,
      body: { sha: newCommitSha, force: false }
    });

    const pullRequest = await github<PullRequestResponse>({
      method: "POST",
      owner: input.owner,
      repo: input.repo,
      path: "/pulls",
      body: {
        title: input.title,
        body: input.body,
        head: input.branchName,
        base: input.baseBranch,
        draft: input.draft
      }
    });

    return {
      number: pullRequest.number,
      url: pullRequest.html_url,
      branch: input.branchName,
      commit: newCommitSha
    };
  }
});
