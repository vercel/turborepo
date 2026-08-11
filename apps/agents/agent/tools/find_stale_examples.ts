import { defineTool } from "eve/tools";
import { z } from "zod";

import {
  getLastCommitForPath,
  listOpenPullRequests,
  listPullRequestFiles,
  resolveRepository
} from "../lib/github.js";
import { listExampleNames } from "../lib/repo.js";

interface PullRequestReference {
  number: number;
  title: string;
  url: string;
  headRef: string;
}

interface ExampleStatus {
  example: string;
  lastCommitAt: string | null;
  lastCommitSha: string | null;
  daysSinceUpdate: number | null;
  openPullRequests: PullRequestReference[];
}

interface SkippedExample extends ExampleStatus {
  reason: "open-pull-request" | "unknown-history" | "updated-recently";
}

const MILLISECONDS_PER_DAY = 86_400_000;
const REQUEST_CONCURRENCY = 8;
const MAX_PULL_REQUEST_FILES = 300;

export default defineTool({
  description:
    "Find examples that are stale and not already covered by an open pull request. An example is stale when examples/<name> has had no commit within the staleness window (one week by default). Examples with an open pull request touching their directory are skipped so the agent never opens a duplicate. Use this before updating examples, and give each returned queue entry its own workflow run.",
  inputSchema: z.object({
    owner: z
      .string()
      .min(1)
      .optional()
      .describe("Repository owner. Defaults to GITHUB_REPOSITORY."),
    repo: z
      .string()
      .min(1)
      .optional()
      .describe("Repository name. Defaults to GITHUB_REPOSITORY."),
    baseBranch: z
      .string()
      .min(1)
      .default("main")
      .describe("Branch to read example commit history from."),
    staleAfterDays: z
      .number()
      .int()
      .positive()
      .max(365)
      .default(7)
      .describe(
        "How many days without a commit make an example stale. Defaults to one week."
      ),
    maxOpenPullRequests: z
      .number()
      .int()
      .positive()
      .max(300)
      .default(100)
      .describe(
        "How many open pull requests to inspect, most recently updated first."
      )
  }),
  async execute({
    owner,
    repo,
    baseBranch,
    staleAfterDays,
    maxOpenPullRequests
  }) {
    const repository = resolveRepository({ owner, repo });
    const examples = await listExampleNames();
    const checkedAt = new Date();
    const staleBefore = new Date(
      checkedAt.getTime() - staleAfterDays * MILLISECONDS_PER_DAY
    );

    const pullRequests = await listOpenPullRequests({
      ...repository,
      limit: maxOpenPullRequests
    });
    const pullRequestsByExample = await mapPullRequestsToExamples(
      repository,
      pullRequests,
      new Set(examples)
    );

    const statuses = await mapWithConcurrency(examples, async (example) => {
      const commit = await getLastCommitForPath({
        ...repository,
        path: `examples/${example}`,
        ref: baseBranch
      });
      const committedAt = commit?.committedAt
        ? Date.parse(commit.committedAt)
        : Number.NaN;

      return {
        example,
        lastCommitAt: commit?.committedAt ?? null,
        lastCommitSha: commit?.sha ?? null,
        daysSinceUpdate: Number.isNaN(committedAt)
          ? null
          : Math.floor(
              (checkedAt.getTime() - committedAt) / MILLISECONDS_PER_DAY
            ),
        openPullRequests: pullRequestsByExample.get(example) ?? []
      } satisfies ExampleStatus;
    });

    const updateQueue: ExampleStatus[] = [];
    const skipped: SkippedExample[] = [];

    for (const status of statuses) {
      if (status.openPullRequests.length > 0) {
        skipped.push({ ...status, reason: "open-pull-request" });
        continue;
      }
      if (status.lastCommitAt === null) {
        skipped.push({ ...status, reason: "unknown-history" });
        continue;
      }
      if (Date.parse(status.lastCommitAt) >= staleBefore.getTime()) {
        skipped.push({ ...status, reason: "updated-recently" });
        continue;
      }
      updateQueue.push(status);
    }

    updateQueue.sort(
      (a, b) =>
        Date.parse(a.lastCommitAt ?? "") - Date.parse(b.lastCommitAt ?? "")
    );

    return {
      repository: `${repository.owner}/${repository.repo}`,
      baseBranch,
      staleAfterDays,
      staleBefore: staleBefore.toISOString(),
      checkedAt: checkedAt.toISOString(),
      inspectedPullRequests: pullRequests.length,
      updateQueue,
      skipped: skipped.sort((a, b) => a.example.localeCompare(b.example))
    };
  }
});

async function mapPullRequestsToExamples(
  repository: { owner: string; repo: string },
  pullRequests: Awaited<ReturnType<typeof listOpenPullRequests>>,
  examples: Set<string>
): Promise<Map<string, PullRequestReference[]>> {
  const byExample = new Map<string, PullRequestReference[]>();

  await mapWithConcurrency(pullRequests, async (pullRequest) => {
    const files = await listPullRequestFiles({
      ...repository,
      pullNumber: pullRequest.number,
      maxFiles: MAX_PULL_REQUEST_FILES
    });

    for (const example of exampleNamesFromFiles(files, examples)) {
      const references = byExample.get(example) ?? [];
      references.push({
        number: pullRequest.number,
        title: pullRequest.title,
        url: pullRequest.url,
        headRef: pullRequest.headRef
      });
      byExample.set(example, references);
    }
  });

  return byExample;
}

function exampleNamesFromFiles(
  files: string[],
  examples: Set<string>
): Set<string> {
  const names = new Set<string>();
  for (const file of files) {
    const [root, example] = file.split("/");
    if (root === "examples" && example && examples.has(example)) {
      names.add(example);
    }
  }
  return names;
}

async function mapWithConcurrency<Item, Result>(
  items: Item[],
  handler: (item: Item) => Promise<Result>
): Promise<Result[]> {
  const results = Array.from<Result>({ length: items.length });
  let cursor = 0;

  async function worker() {
    while (cursor < items.length) {
      const index = cursor;
      cursor += 1;
      results[index] = await handler(items[index]);
    }
  }

  await Promise.all(
    Array.from({ length: Math.min(REQUEST_CONCURRENCY, items.length) }, worker)
  );

  return results;
}
