export interface PullRequestInput {
  title: string;
  body: string;
  head: string;
  base: string;
}

/**
 * `.github/workflows/lint-pr-title.yml` enforces Conventional Commit titles
 * with an uppercase description and no scope.
 */
export const CONVENTIONAL_TITLE_PATTERN = /^[a-z]+: [A-Z].+$/;
const PERFORMANCE_TITLE_PATTERN = /^perf: [A-Z].+$/;

export interface BranchRefUpdate {
  readonly afterOid: string;
  readonly beforeOid: string;
  readonly force: true;
  readonly name: string;
}

export interface PullRequestNaming {
  /** Set for an automated example-maintenance run, which titles itself. */
  readonly automatedExample?: string;
  /** Set when the pull request changes the Factory, which always uses `chore:`. */
  readonly factory?: boolean;
  /** Set for an automated performance run, which must publish a `perf:` title. */
  readonly performance?: boolean;
  /** Title supplied by the caller of an interactive run. */
  readonly requestedTitle?: string;
}

export function buildDraftPullRequest(input: PullRequestInput) {
  return { ...input, draft: true as const };
}

export function formatPullRequestSlackNotification(
  title: string,
  url: string
): string {
  return `:pr: *<${url}|${title}>*`;
}

export function formatMergedPullRequestSlackNotification(
  title: string,
  url: string
): string {
  return `:pr-merged: *<${url}|${title}>*`;
}

export function mergedFactoryPullRequest(
  action: string,
  raw: Readonly<Record<string, unknown>>
): { readonly title: string; readonly url: string } | null {
  const pullRequest = raw.pull_request;
  if (
    action !== "closed" ||
    typeof pullRequest !== "object" ||
    pullRequest === null
  ) {
    return null;
  }

  const value = pullRequest as Readonly<Record<string, unknown>>;
  const head = value.head;
  if (typeof head !== "object" || head === null) return null;
  const branch = (head as Readonly<Record<string, unknown>>).ref;
  return value.merged === true &&
    typeof branch === "string" &&
    branch.startsWith("agents/") &&
    typeof value.title === "string" &&
    typeof value.html_url === "string"
    ? { title: value.title, url: value.html_url }
    : null;
}

export function buildBranchRefUpdate(
  branchName: string,
  beforeOid: string,
  afterOid: string
): BranchRefUpdate {
  return {
    afterOid,
    beforeOid,
    force: true,
    name: `refs/heads/${branchName}`
  };
}

export async function updateBranchRefWithLease(
  input: {
    branchName: string;
    expectedSha: string;
    newSha: string;
    repositoryId: string;
    token: string;
  },
  fetchImplementation: typeof fetch = fetch
): Promise<void> {
  const response = await fetchImplementation("https://api.github.com/graphql", {
    method: "POST",
    headers: {
      accept: "application/vnd.github+json",
      authorization: `Bearer ${input.token}`,
      "content-type": "application/json",
      "x-github-api-version": "2022-11-28"
    },
    body: JSON.stringify({
      query: `mutation UpdateBranchWithLease($input: UpdateRefsInput!) {
        updateRefs(input: $input) { clientMutationId }
      }`,
      variables: {
        input: {
          repositoryId: input.repositoryId,
          refUpdates: [
            buildBranchRefUpdate(
              input.branchName,
              input.expectedSha,
              input.newSha
            )
          ]
        }
      }
    })
  });

  if (!response.ok) {
    throw new Error(
      `GitHub GraphQL request failed with ${response.status}: ${await response.text()}`
    );
  }

  const body = (await response.json()) as {
    data?: { updateRefs?: unknown };
    errors?: Array<{ message?: unknown }>;
  };
  if (body.errors?.length) {
    const detail = body.errors
      .map((error) => String(error.message ?? "Unknown GraphQL error"))
      .join("; ");
    throw new Error(`GitHub GraphQL request failed: ${detail}`);
  }
  if (body.data?.updateRefs === undefined || body.data.updateRefs === null) {
    throw new Error("GitHub GraphQL response did not confirm the ref update.");
  }
}

/**
 * Resolves the commit and pull request title for a run. Automated maintenance
 * derives its own title from the example it selected; every other run has to
 * name its change, because it can touch anything in the checkout.
 */
export function resolvePullRequestTitle(naming: PullRequestNaming): string {
  if (naming.performance) {
    if (
      naming.requestedTitle === undefined ||
      !PERFORMANCE_TITLE_PATTERN.test(naming.requestedTitle)
    ) {
      throw new Error(
        "Automated performance pull requests require a 'perf: Description' title."
      );
    }
    return naming.requestedTitle;
  }

  if (naming.automatedExample !== undefined) {
    return `chore: Update ${naming.automatedExample} example`;
  }

  if (
    naming.requestedTitle === undefined ||
    !CONVENTIONAL_TITLE_PATTERN.test(naming.requestedTitle)
  ) {
    throw new Error(
      "Interactive pull requests require a Conventional Commit title such as 'fix: Correct the task hash'."
    );
  }
  return naming.factory
    ? naming.requestedTitle.replace(/^[a-z]+:/, "chore:")
    : naming.requestedTitle;
}
