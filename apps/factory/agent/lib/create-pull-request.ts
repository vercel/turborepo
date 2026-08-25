import type { SandboxSession } from "eve/sandbox";

import { requireSuccessfulValidation } from "./example-validation";
import { getGitHubToken } from "./github";
import {
  readPerformanceState,
  requirePublishablePerformanceChange
} from "./performance-validation";
import {
  buildDraftPullRequest,
  resolvePullRequestTitle,
  updateBranchRefWithLease
} from "./pull-request";
import {
  assertNoReleaseAgeExclusion,
  isReleaseAgeConfigFile
} from "./release-age";
import { isAppPrincipal, resolveAutomatedSelection } from "./repo";

const owner = "vercel";
const repo = "turborepo";
const baseBranch = "main";
const checkout = "turborepo";

export interface CreatePullRequestInput {
  readonly body: string;
  readonly branchName?: string;
  readonly title?: string;
}

export interface CreatePullRequestContext {
  readonly auth: Parameters<typeof isAppPrincipal>[0];
  readonly sandbox: SandboxSession;
  readonly sessionId: string;
}

type RefResponse = { object?: { sha?: string } };
type CommitResponse = { tree?: { sha?: string } };
type CompareResponse = { merge_base_commit?: { sha?: string } };
type ShaResponse = { sha?: string };
type RepositoryResponse = { node_id?: string };
type PullRequestResponse = {
  head?: { sha?: string };
  html_url?: string;
  number?: number;
};
type ValidatedPullRequest = {
  number: number;
  url: string;
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

function requirePullRequest(
  response: PullRequestResponse
): ValidatedPullRequest {
  if (
    typeof response.number !== "number" ||
    !Number.isSafeInteger(response.number) ||
    response.number <= 0
  ) {
    throw new Error(
      "GitHub response did not include a valid pull request number."
    );
  }

  const number = response.number;
  const expectedUrl = `https://github.com/${owner}/${repo}/pull/${number}`;
  if (response.html_url !== expectedUrl) {
    throw new Error(
      "GitHub response did not include the expected pull request URL."
    );
  }

  return { number, url: expectedUrl };
}

async function findOpenPullRequests(branchName: string) {
  return github<PullRequestResponse[]>({
    method: "GET",
    owner,
    repo,
    path: `/pulls?state=open&head=${owner}%3A${encodeURIComponent(branchName)}`
  });
}

async function reconcileCreatedPullRequest(
  response: PullRequestResponse,
  branchName: string,
  headSha: string
): Promise<ValidatedPullRequest> {
  try {
    return requirePullRequest(response);
  } catch {
    const pullRequest = (await findOpenPullRequests(branchName)).find(
      (candidate) => candidate.head?.sha === headSha
    );
    if (!pullRequest) {
      throw new Error(
        "GitHub pull request response was invalid and could not be reconciled."
      );
    }
    return requirePullRequest(pullRequest);
  }
}

async function github<T>(input: {
  body?: unknown;
  method: "GET" | "POST";
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

async function publishBranchUpdateWithLease(input: {
  branchName: string;
  expectedSha: string;
  newSha: string;
}) {
  const repository = await github<RepositoryResponse>({
    method: "GET",
    owner,
    repo,
    path: ""
  });
  if (typeof repository.node_id !== "string" || repository.node_id === "") {
    throw new Error("GitHub response did not include repository node ID.");
  }
  await updateBranchRefWithLease({
    ...input,
    repositoryId: repository.node_id,
    token: await getGitHubToken()
  });
}

export async function createPullRequest(
  input: CreatePullRequestInput,
  context: CreatePullRequestContext
) {
  const { auth, sandbox, sessionId } = context;
  const automated = isAppPrincipal(auth);
  const performanceState = automated
    ? await readPerformanceState(sandbox, sessionId)
    : null;
  const selection =
    automated && auth && !performanceState
      ? await resolveAutomatedSelection(sandbox, auth, sessionId)
      : null;
  const changedPaths = await listAllChangedPaths(sandbox);
  if (selection) {
    const expectedPrefix = `examples/${selection.example}/`;
    const unexpectedFile = changedPaths.find(
      (file) => !file.startsWith(expectedPrefix)
    );
    if (unexpectedFile) {
      throw new Error(
        `Automated maintenance for ${selection.example} cannot publish ${unexpectedFile}.`
      );
    }
  }
  if (performanceState) {
    const forbidden = changedPaths.find(isForbiddenPerformancePath);
    if (forbidden) {
      throw new Error(
        `Automated performance improvements cannot publish ${forbidden}.`
      );
    }
    await requirePublishablePerformanceChange(sandbox, sessionId);
  }
  // Automated example maintenance publishes only its selected example. Every
  // other run — a performance run, or an interactive request from an operator
  // — publishes whatever it changed in the checkout.
  const changedFiles = await listChangedFiles(sandbox, selection === null);
  if (changedFiles.length === 0) {
    return { created: false, reason: "No publishable changes." };
  }
  if (selection) {
    await requireSuccessfulValidation(sandbox, sessionId, selection.example);
  }
  const branchName = performanceState
    ? `agents/performance-${performanceState.date}`
    : selection
      ? `agents/examples-${selection.example}-${selection.date}`
      : input.branchName;
  if (!branchName) {
    throw new Error("Interactive pull requests require an agents/* branch.");
  }
  const changeTitle = resolvePullRequestTitle({
    automatedExample: selection?.example,
    performance: performanceState !== null,
    requestedTitle: input.title
  });

  const existingPullRequests = await findOpenPullRequests(branchName);
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
      if (
        file.path.startsWith("examples/") &&
        isReleaseAgeConfigFile(file.path)
      ) {
        assertNoReleaseAgeExclusion(
          file.path,
          Buffer.from(content).toString("utf8")
        );
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
      reason: "No effective changes against main."
    };
  }

  if (existingPullRequest) {
    const validatedPullRequest = requirePullRequest(existingPullRequest);
    const headSha = requireSha(
      existingPullRequest.head?.sha,
      "pull request head SHA"
    );
    const existingCommit = await github<CommitResponse>({
      method: "GET",
      owner,
      repo,
      path: `/git/commits/${headSha}`
    });
    if (existingCommit.tree?.sha === newTreeSha) {
      return {
        created: false,
        existing: true,
        number: validatedPullRequest.number,
        url: validatedPullRequest.url,
        branch: branchName,
        commit: headSha
      };
    }

    const commit = await github<ShaResponse>({
      method: "POST",
      owner,
      repo,
      path: "/git/commits",
      body: {
        message: changeTitle,
        tree: newTreeSha,
        parents: [checkoutSha]
      }
    });
    const updatedCommitSha = requireSha(commit.sha, "commit SHA");
    // updateRefs applies beforeOid atomically, matching the safety of
    // `git push --force-with-lease` without exposing the installation token
    // inside the sandbox.
    await publishBranchUpdateWithLease({
      branchName,
      expectedSha: headSha,
      newSha: updatedCommitSha
    });

    return {
      created: false,
      existing: true,
      updated: true,
      number: validatedPullRequest.number,
      url: validatedPullRequest.url,
      branch: branchName,
      commit: updatedCommitSha
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
        message: changeTitle,
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
    body: buildDraftPullRequest({
      title: changeTitle,
      body: input.body,
      head: branchName,
      base: baseBranch
    })
  });
  const validatedPullRequest = await reconcileCreatedPullRequest(
    pullRequest,
    branchName,
    newCommitSha
  );

  const { deliverSlackMessage } = await import("./slack");
  const slackNotification = await deliverSlackMessage(
    `A new Turborepo pull request was created: #${validatedPullRequest.number} ${validatedPullRequest.url}`,
    {
      event: "pull_request_notification",
      metadata: { pullRequestNumber: validatedPullRequest.number }
    }
  );

  return {
    created: true,
    number: validatedPullRequest.number,
    url: validatedPullRequest.url,
    branch: branchName,
    commit: newCommitSha,
    slackNotification
  };
}

async function listChangedFiles(
  sandbox: SandboxSession,
  repositoryWide = false
): Promise<
  Array<{ path: string; deleted: boolean; mode: "100644" | "100755" }>
> {
  const scope = repositoryWide ? "" : " -- examples";
  const modified = await runGit(
    sandbox,
    `git diff --no-renames --name-only --diff-filter=ACMRTUXB HEAD${scope}`
  );
  const deleted = await runGit(
    sandbox,
    `git diff --no-renames --name-only --diff-filter=D HEAD${scope}`
  );
  const untracked = await runGit(
    sandbox,
    `git ls-files --others --exclude-standard${scope}`
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

async function listAllChangedPaths(sandbox: SandboxSession): Promise<string[]> {
  const tracked = await runGit(
    sandbox,
    "git diff --no-renames --name-only HEAD"
  );
  const untracked = await runGit(
    sandbox,
    "git ls-files --others --exclude-standard"
  );
  return [
    ...new Set(
      [tracked, untracked].flatMap((output) =>
        output.split("\n").filter(Boolean)
      )
    )
  ].sort();
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
      command: `root=$(realpath .) && resolved=$(realpath -- ${shellQuote(file)}) && case "$resolved" in "$root"/*) test -f "$resolved" && test ! -L ${shellQuote(file)} ;; *) exit 1 ;; esac`,
      workingDirectory: checkout
    });
    if (validation.exitCode !== 0) {
      throw new Error(`Changed path ${file} is not a regular repository file.`);
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

function isForbiddenPerformancePath(file: string): boolean {
  return (
    file === "Cargo.lock" ||
    file === "pnpm-lock.yaml" ||
    file.startsWith(".github/") ||
    file.startsWith("apps/factory/") ||
    file.startsWith(".changeset/") ||
    file.startsWith("target/") ||
    file === "version.txt"
  );
}

function shellQuote(value: string): string {
  return `'${value.replaceAll("'", `'\\''`)}'`;
}
