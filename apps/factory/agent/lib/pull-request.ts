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

export interface PullRequestNaming {
  /** Set for an automated example-maintenance run, which titles itself. */
  readonly automatedExample?: string;
  /** Set for an automated performance run, which must publish a `perf:` title. */
  readonly performance?: boolean;
  /** Title supplied by the caller of an interactive run. */
  readonly requestedTitle?: string;
}

export function buildDraftPullRequest(input: PullRequestInput) {
  return { ...input, draft: true as const };
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
  return naming.requestedTitle;
}
