import { defineTool } from "eve/tools";
import { always } from "eve/tools/approval";
import { z } from "zod";

import { GitHubApiError, githubRequest } from "../lib/github.js";

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
    .regex(/^agents\/[A-Za-z0-9._/-]+$/, "Branch must start with agents/"),
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
type PullRequestResponse = { html_url?: string; number?: number };

function requireSha(value: string | undefined, label: string) {
  if (!value) throw new Error(`GitHub response did not include ${label}.`);
  return value;
}

export default defineTool({
  description:
    "Create a GitHub pull request from selected sandbox files. Open at most one pull request per example, on the branch agents/examples/<example>. Use only after editing files in the sandbox and confirming the example has no open pull request already.",
  inputSchema,
  approval: always(),
  async execute(input, ctx) {
    const sandbox = await ctx.getSandbox();
    const baseRef = await githubRequest<RefResponse>({
      method: "GET",
      owner: input.owner,
      repo: input.repo,
      path: `/git/ref/heads/${input.baseBranch}`
    });
    const baseCommitSha = requireSha(baseRef.object?.sha, "base ref SHA");

    let branchCommitSha = baseCommitSha;
    try {
      const branchRef = await githubRequest<RefResponse>({
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

      await githubRequest<RefResponse>({
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

    const branchCommit = await githubRequest<CommitResponse>({
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
        const blob = await githubRequest<ShaResponse>({
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

    const newTree = await githubRequest<ShaResponse>({
      method: "POST",
      owner: input.owner,
      repo: input.repo,
      path: "/git/trees",
      body: {
        base_tree: baseTreeSha,
        tree
      }
    });

    const commit = await githubRequest<ShaResponse>({
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

    await githubRequest<RefResponse>({
      method: "PATCH",
      owner: input.owner,
      repo: input.repo,
      path: `/git/refs/heads/${input.branchName}`,
      body: { sha: newCommitSha, force: false }
    });

    const pullRequest = await githubRequest<PullRequestResponse>({
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
