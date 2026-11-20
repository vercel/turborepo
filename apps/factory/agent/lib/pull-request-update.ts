export type ExistingPullRequestUpdate = "unchanged" | "update";

/**
 * An update may only extend the exact PR head that the sandbox checked out.
 * GitHub's non-forced ref update supplies the final race check after this plan.
 */
export function resolveExistingPullRequestUpdate(input: {
  readonly checkoutSha: string;
  readonly currentTreeSha: string | undefined;
  readonly headSha: string;
  readonly newTreeSha: string;
  readonly pullRequestUrl: string;
}): ExistingPullRequestUpdate {
  if (input.currentTreeSha === input.newTreeSha) return "unchanged";
  if (input.headSha !== input.checkoutSha) {
    throw new Error(
      `Pull request ${input.pullRequestUrl} changed after this checkout or its branch already contains different changes.`
    );
  }
  return "update";
}
