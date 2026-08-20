/**
 * GitHub push webhook filtering.
 *
 * The Eve GitHub channel serves `/eve/v1/github` and only dispatches
 * comment and CI events, so merges to `main` are handled by this
 * application's own Connect-forwarded endpoint instead of a GitHub Actions
 * job. Everything here is pure so `tests/github-push.test.mjs` can exercise
 * event filtering without a live delivery.
 */

export const TURBOREPO_REPOSITORY = "vercel/turborepo";
export const MAIN_REF = "refs/heads/main";

const COMMIT_PATTERN = /^[0-9a-f]{40}$/;
const EMPTY_COMMIT = "0".repeat(40);

export interface GitHubPush {
  readonly commit: string;
  readonly pusher?: string;
  readonly ref: string;
}

export type GitHubPushOutcome =
  | { readonly kind: "ignored"; readonly reason: string }
  | { readonly kind: "ping" }
  | { readonly kind: "push"; readonly push: GitHubPush };

/** Only the configured Connect connector may submit factory image pushes. */
export function isFactoryImageConnector(
  auth:
    | {
        readonly attributes: Readonly<
          Record<string, string | readonly string[]>
        >;
      }
    | null
    | undefined,
  connectorId = process.env.FACTORY_IMAGE_CONNECTOR_ID
): boolean {
  return (
    connectorId !== undefined && auth?.attributes.connector_id === connectorId
  );
}

/**
 * Decides whether one delivery should rebuild the factory image. Only a
 * non-deleting push that leaves a real commit on `main` of the Turborepo
 * repository qualifies.
 */
export function parseGitHubPush(
  event: string | null | undefined,
  body: string,
  repository = TURBOREPO_REPOSITORY
): GitHubPushOutcome {
  if (event === "ping") return { kind: "ping" };
  if (event !== null && event !== undefined && event !== "push") {
    return {
      kind: "ignored",
      reason: `Unsupported event: ${event}.`
    };
  }

  let payload: unknown;
  try {
    payload = JSON.parse(body);
  } catch {
    return { kind: "ignored", reason: "The payload is not valid JSON." };
  }
  if (
    typeof payload !== "object" ||
    payload === null ||
    Array.isArray(payload)
  ) {
    return { kind: "ignored", reason: "The payload is not an object." };
  }

  const push = payload as Record<string, unknown>;
  // Connect forwarding may omit GitHub's event header. Only infer a push
  // from the two fields that identify its resulting revision.
  if (event === null || event === undefined) {
    if (!("ref" in push && "after" in push)) {
      return { kind: "ignored", reason: "Unsupported event: none." };
    }
  }
  const fullName = (push.repository as { full_name?: unknown } | undefined)
    ?.full_name;
  if (fullName !== repository) {
    return {
      kind: "ignored",
      reason: `Unexpected repository: ${String(fullName)}.`
    };
  }
  if (push.ref !== MAIN_REF) {
    return { kind: "ignored", reason: `Not a ${MAIN_REF} push.` };
  }
  if (push.deleted === true) {
    return { kind: "ignored", reason: "The branch was deleted." };
  }

  const commit = push.after;
  if (
    typeof commit !== "string" ||
    !COMMIT_PATTERN.test(commit) ||
    commit === EMPTY_COMMIT
  ) {
    return { kind: "ignored", reason: "The push has no head commit." };
  }

  const pusher = (push.pusher as { name?: unknown } | undefined)?.name;
  return {
    kind: "push",
    push: {
      commit,
      pusher: typeof pusher === "string" ? pusher : undefined,
      ref: MAIN_REF
    }
  };
}
