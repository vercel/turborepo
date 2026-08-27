export const FACTORY_PULL_REQUEST_BRANCH_ATTRIBUTE =
  "factory_pull_request_branch";
export const FACTORY_PULL_REQUEST_FEEDBACK_ATTRIBUTE =
  "factory_pull_request_feedback";

interface FactoryPullRequestFeedbackCandidate {
  readonly branch: unknown;
  readonly conversationKind: string;
  readonly permission: unknown;
  readonly pullRequestNumber: number | null;
  readonly repository: string;
  readonly senderType: string;
}

const WRITE_PERMISSIONS = new Set(["admin", "maintain", "write"]);

export function hasGitHubInvocation(body: string, botName: string): boolean {
  const escapedBotName = botName.replace(/[.*+?^${}()|[\]\\]/g, "\\$&");
  return new RegExp(`@${escapedBotName}(?=$|[^A-Za-z0-9_-])`, "iu").test(body);
}

/**
 * Factory PR feedback runs without an invocation token, so this is deliberately
 * fail-closed: only human collaborators with write access can dispatch turns,
 * and only on PRs backed by a Factory-owned agents/* branch.
 */
export function isTrustedFactoryPullRequestFeedback(
  candidate: FactoryPullRequestFeedbackCandidate
): candidate is FactoryPullRequestFeedbackCandidate & {
  readonly branch: string;
} {
  return (
    candidate.repository === "vercel/turborepo" &&
    candidate.conversationKind === "review_thread" &&
    candidate.pullRequestNumber !== null &&
    candidate.senderType !== "Bot" &&
    typeof candidate.branch === "string" &&
    /^agents\/[A-Za-z0-9._/-]+$/.test(candidate.branch) &&
    typeof candidate.permission === "string" &&
    WRITE_PERMISSIONS.has(candidate.permission)
  );
}

export function isAuthorizedFactoryPullRequestUpdate(
  auth:
    | {
        readonly authenticator?: string;
        readonly attributes?: Readonly<
          Record<string, string | readonly string[]>
        >;
      }
    | null
    | undefined,
  branchName: string | undefined
): boolean {
  return (
    auth?.authenticator === "github-webhook" &&
    auth.attributes?.[FACTORY_PULL_REQUEST_FEEDBACK_ATTRIBUTE] === "true" &&
    typeof branchName === "string" &&
    auth.attributes[FACTORY_PULL_REQUEST_BRANCH_ATTRIBUTE] === branchName
  );
}
