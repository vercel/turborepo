/**
 * Browser access to the eve session routes for the operator console's chat.
 *
 * Vercel Deployment Protection decides who reaches this app at all, exactly as
 * it does for the operator run routes. What it cannot tell the agent is whether
 * a request came from the console page or from another site riding the
 * operator's protection cookie, so the console marks every eve request with
 * `x-operator-action` and this check refuses anything that arrives without the
 * marker, declares a different origin, or announces a cross-site fetch. The eve
 * session routes answer with no CORS headers, so a cross-origin caller cannot
 * get the marker past a preflight either.
 */

// Sent as `x-operator-action` by the console's chat and required by the eve
// channel, so both sides of the contract stay in sync.
export const OPERATOR_ACTION_HEADER = "x-operator-action";
export const OPERATOR_CHAT_ACTION = "open-operator-chat";

interface OperatorConsoleRequest {
  readonly headers: { get: (name: string) => string | null };
}

function originHost(origin: string): string | null {
  try {
    return new URL(origin).host;
  } catch {
    return null;
  }
}

/**
 * Principal for a console chat turn. It is deliberately *not* the app
 * principal the schedules and run routes use, so `create_pull_request` keeps
 * asking the operator for approval and the automated scope gates stay off.
 */
export const OPERATOR_CHAT_PRINCIPAL = {
  attributes: {},
  authenticator: "operator-console",
  principalId: "turborepo-factory-operator",
  principalType: "user"
} as const;

export function isOperatorChatRequest(
  request: OperatorConsoleRequest
): boolean {
  if (request.headers.get(OPERATOR_ACTION_HEADER) !== OPERATOR_CHAT_ACTION) {
    return false;
  }

  // Set by the browser and unforgeable from page script. Non-browser callers
  // omit it, and for them the marker is the whole check.
  const site = request.headers.get("sec-fetch-site");
  if (site !== null && site !== "same-origin") {
    return false;
  }

  // Browsers omit `origin` on a same-origin stream GET. When it is there it has
  // to name the host the browser addressed, which is the forwarded host rather
  // than `request.url`: both Vercel and `next start` route these paths to the
  // eve service through a proxy that rewrites the host it dials.
  const origin = request.headers.get("origin");
  if (origin === null) return true;
  const host =
    request.headers.get("x-forwarded-host") ?? request.headers.get("host");
  return host !== null && originHost(origin) === host;
}
