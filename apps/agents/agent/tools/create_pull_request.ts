import type { SandboxSession } from "eve/sandbox";
import { defineTool } from "eve/tools";
import { z } from "zod";

import { getGitHubToken } from "../lib/github.js";

const owner = "vercel";
const repo = "turborepo";
const baseBranch = "main";
const checkout = "turborepo";

const inputSchema = z.object({
  branchName: z
    .string()
    .regex(/^agents\/[A-Za-z0-9._/-]+$/, "Branch must start with agents/")
    .optional()
    .describe(
      "Branch for an interactive run. Scheduled runs always use the current UTC date."
    ),
  body: z.string().default("")
});

type RefResponse = { object?: { sha?: string } };
type CommitResponse = { tree?: { sha?: string } };
type CompareResponse = { merge_base_commit?: { sha?: string } };
type ShaResponse = { sha?: string };
type PullRequestResponse = {
  head?: { sha?: string };
  html_url?: string;
  number?: number;
};
type TreeEntry = {
  path: string;
  mode: "100644" | "100755";
  type: "blob";
  sha: string | null;
};

class GitHubApiError extends Error {
  constructor(
    message: string,
    readonly status: number
  ) {
    super(message);
  }
}

function repoPath(owner: string, repo: string, path: string) {
  return `/repos/${encodeURIComponent(owner)}/${encodeURIComponent(repo)}${path}`;
}

function requireSha(value: string | undefined, label: string) {
  if (!value) throw new Error(`GitHub response did not include ${label}.`);
  return value;
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
        authorization: `Bearer ${await getGitHubToken()}`,
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
    "Create a vercel/turborepo pull request containing every sandbox change under examples/. Returns without creating a PR when there are no changes.",
  inputSchema,
  approval: ({ session }) => {
    return isAppPrincipal(session.auth.current)
      ? "not-applicable"
      : "user-approval";
  },
  async execute(input, ctx) {
    const sandbox = await ctx.getSandbox();
    const branchName = isAppPrincipal(ctx.session.auth.current)
      ? scheduledBranchName()
      : input.branchName;
    if (!branchName) {
      throw new Error("Interactive pull requests require an agents/* branch.");
    }
    const changedFiles = await listChangedFiles(sandbox);
    if (changedFiles.length === 0) {
      return { created: false, reason: "No changes under examples/." };
    }

    const existingPullRequests = await github<PullRequestResponse[]>({
      method: "GET",
      owner,
      repo,
      path: `/pulls?state=open&head=${owner}%3A${encodeURIComponent(branchName)}`
    });
    const existingPullRequest = existingPullRequests[0];

    const checkoutSha = (await runGit(sandbox, "git rev-parse HEAD")).trim();
    const comparison = await github<CompareResponse>({
      method: "GET",
      owner,
      repo,
      path: `/compare/${checkoutSha}...${baseBranch}`
    });
    if (comparison.merge_base_commit?.sha !== checkoutSha) {
      throw new Error(
        "The sandbox checkout is not based on vercel/turborepo main."
      );
    }

    const branchCommit = await github<CommitResponse>({
      method: "GET",
      owner,
      repo,
      path: `/git/commits/${checkoutSha}`
    });
    const baseTreeSha = requireSha(branchCommit.tree?.sha, "base tree SHA");

    const tree: TreeEntry[] = await Promise.all(
      changedFiles.map(async (file) => {
        if (file.deleted) {
          return {
            path: file.path,
            mode: file.mode,
            type: "blob",
            sha: null
          };
        }

        const content = await sandbox.readBinaryFile({
          path: `${checkout}/${file.path}`
        });
        if (content === null) {
          throw new Error(`Changed file ${file.path} does not exist.`);
        }
        const blob = await github<ShaResponse>({
          method: "POST",
          owner,
          repo,
          path: "/git/blobs",
          body: {
            content: Buffer.from(content).toString("base64"),
            encoding: "base64"
          }
        });

        return {
          path: file.path,
          mode: file.mode,
          type: "blob",
          sha: requireSha(blob.sha, "blob SHA")
        };
      })
    );

    const newTree = await github<ShaResponse>({
      method: "POST",
      owner,
      repo,
      path: "/git/trees",
      body: {
        base_tree: baseTreeSha,
        tree
      }
    });

    const newTreeSha = requireSha(newTree.sha, "tree SHA");
    if (newTreeSha === baseTreeSha) {
      return {
        created: false,
        reason: "No effective changes under examples/."
      };
    }

    if (existingPullRequest) {
      const existingCommit = await github<CommitResponse>({
        method: "GET",
        owner,
        repo,
        path: `/git/commits/${requireSha(existingPullRequest.head?.sha, "pull request head SHA")}`
      });
      if (existingCommit.tree?.sha !== newTreeSha) {
        throw new Error(
          `Pull request ${existingPullRequest.html_url ?? existingPullRequest.number} already uses ${branchName} with different changes.`
        );
      }
      return {
        created: false,
        existing: true,
        number: existingPullRequest.number,
        url: existingPullRequest.html_url,
        branch: branchName,
        commit: existingPullRequest.head?.sha
      };
    }

    let newCommitSha: string;
    try {
      const branchRef = await github<RefResponse>({
        method: "GET",
        owner,
        repo,
        path: `/git/ref/heads/${branchName}`
      });
      newCommitSha = requireSha(branchRef.object?.sha, "branch ref SHA");
      const existingCommit = await github<CommitResponse>({
        method: "GET",
        owner,
        repo,
        path: `/git/commits/${newCommitSha}`
      });
      if (existingCommit.tree?.sha !== newTreeSha) {
        throw new Error(
          `Branch ${branchName} already exists with different changes.`
        );
      }
    } catch (error) {
      if (!(error instanceof GitHubApiError) || error.status !== 404) {
        throw error;
      }

      const commit = await github<ShaResponse>({
        method: "POST",
        owner,
        repo,
        path: "/git/commits",
        body: {
          message: "chore: Update Turborepo examples",
          tree: newTreeSha,
          parents: [checkoutSha]
        }
      });
      newCommitSha = requireSha(commit.sha, "commit SHA");

      await github({
        method: "POST",
        owner,
        repo,
        path: "/git/refs",
        body: {
          ref: `refs/heads/${branchName}`,
          sha: newCommitSha
        }
      });
    }

    const pullRequest = await github<PullRequestResponse>({
      method: "POST",
      owner,
      repo,
      path: "/pulls",
      body: {
        title: "chore: Update Turborepo examples",
        body: input.body,
        head: branchName,
        base: baseBranch,
        draft: true
      }
    });

    return {
      created: true,
      number: pullRequest.number,
      url: pullRequest.html_url,
      branch: branchName,
      commit: newCommitSha
    };
  }
});

async function listChangedFiles(
  sandbox: SandboxSession
): Promise<
  Array<{ path: string; deleted: boolean; mode: "100644" | "100755" }>
> {
  const modified = await runGit(
    sandbox,
    "git diff --no-renames --name-only --diff-filter=ACMRTUXB HEAD -- examples"
  );
  const deleted = await runGit(
    sandbox,
    "git diff --no-renames --name-only --diff-filter=D HEAD -- examples"
  );
  const untracked = await runGit(
    sandbox,
    "git ls-files --others --exclude-standard -- examples"
  );
  const deletedFiles = new Set(deleted.split("\n").filter(Boolean));
  const paths = [
    ...new Set(
      [modified, deleted, untracked].flatMap((output) =>
        output.split("\n").filter(Boolean)
      )
    )
  ].sort();

  return Promise.all(
    paths.map(async (file) => ({
      path: file,
      deleted: deletedFiles.has(file),
      mode: await fileMode(sandbox, file)
    }))
  );
}

async function fileMode(
  sandbox: SandboxSession,
  file: string
): Promise<"100644" | "100755"> {
  const deleted = await runGit(
    sandbox,
    `git diff --no-renames --name-only --diff-filter=D HEAD -- ${shellQuote(file)}`
  );
  if (deleted.trim() === "") {
    const validation = await sandbox.run({
      command: `resolved=$(realpath -- ${shellQuote(file)}) && case "$resolved" in /workspace/turborepo/examples/*) test -f "$resolved" && test ! -L ${shellQuote(file)} ;; *) exit 1 ;; esac`,
      workingDirectory: checkout
    });
    if (validation.exitCode !== 0) {
      throw new Error(`Changed path ${file} is not a regular examples file.`);
    }
    const executable = await sandbox.run({
      command: `test -x ${shellQuote(file)}`,
      workingDirectory: checkout
    });
    return executable.exitCode === 0 ? "100755" : "100644";
  }
  const output = await runGit(
    sandbox,
    `git ls-files --stage -- ${shellQuote(file)}`
  );
  if (output.startsWith("120000 ")) {
    throw new Error(`Changed path ${file} is a symbolic link.`);
  }
  return output.startsWith("100755 ") ? "100755" : "100644";
}

async function runGit(
  sandbox: SandboxSession,
  command: string
): Promise<string> {
  const result = await sandbox.run({ command, workingDirectory: checkout });
  if (result.exitCode !== 0) {
    throw new Error(`${command} failed: ${result.stderr}`);
  }
  return result.stdout;
}

function shellQuote(value: string): string {
  return `'${value.replaceAll("'", `'\\''`)}'`;
}

function scheduledBranchName(): string {
  return `agents/weekly-examples-${new Date().toISOString().slice(0, 10)}`;
}

function isAppPrincipal(
  auth:
    | {
        authenticator: string;
        principalId: string;
        principalType: string;
      }
    | null
    | undefined
): boolean {
  return (
    auth?.authenticator === "app" &&
    auth.principalId === "eve:app" &&
    auth.principalType === "runtime"
  );
}
