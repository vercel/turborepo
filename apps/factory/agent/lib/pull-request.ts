export interface PullRequestInput {
  title: string;
  body: string;
  head: string;
  base: string;
}

export function buildDraftPullRequest(input: PullRequestInput) {
  return { ...input, draft: true as const };
}
