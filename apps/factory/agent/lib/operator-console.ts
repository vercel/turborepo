/**
 * Browser access to the Eve session routes used by durable workspaces.
 *
 * Vercel Deployment Protection decides who reaches this app at all. The
 * browser additionally marks every Eve request with `x-operator-action`, and
 * this check refuses anything that arrives without the marker, declares a
 * different origin, or announces a cross-site fetch. The Eve session routes
 * answer with no CORS headers, so a cross-origin caller cannot get the marker
 * past a preflight either.
 */

import type { WorkspaceHarness } from "./workspace";

const WORKSPACE_HARNESS_IDS = new Set<WorkspaceHarness>([
  "fx",
  "claude-code",
  "codex",
  "cursor",
  "opencode",
  "pi"
]);

function isWorkspaceHarness(value: unknown): value is WorkspaceHarness {
  return (
    typeof value === "string" &&
    WORKSPACE_HARNESS_IDS.has(value as WorkspaceHarness)
  );
}

export const OPERATOR_ACTION_HEADER = "x-operator-action";
export const OPERATOR_SESSION_ACTION = "access-workspace-session";

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
 * Principal for an operator-controlled workspace. It is deliberately *not*
 * the app principal used by schedules, so automated scope gates stay off while
 * trusted operator actions can bypass redundant human approval.
 */
export const OPERATOR_SESSION_PRINCIPAL = {
  attributes: {},
  authenticator: "operator-console",
  principalId: "turborepo-factory-operator",
  principalType: "user"
} as const;

export function operatorSessionPrincipal(
  model?: string,
  harness?: WorkspaceHarness
) {
  const attributes: Record<string, string> = {};
  if (
    model !== undefined &&
    /^[a-z0-9][a-z0-9._-]*\/[a-z0-9][a-z0-9._:-]*$/i.test(model)
  ) {
    attributes.selectedModel = model;
  }
  if (harness !== undefined && isWorkspaceHarness(harness)) {
    attributes.selectedHarness = harness;
  }
  return Object.keys(attributes).length === 0
    ? OPERATOR_SESSION_PRINCIPAL
    : { ...OPERATOR_SESSION_PRINCIPAL, attributes };
}

interface OperatorAuth {
  readonly attributes: Readonly<Record<string, string | readonly string[]>>;
}

export function selectedOperatorModel(
  auth: OperatorAuth | null | undefined
): string | undefined {
  const model = auth?.attributes.selectedModel;
  return typeof model === "string" ? model : undefined;
}

export function selectedOperatorHarness(
  auth: OperatorAuth | null | undefined
): WorkspaceHarness | undefined {
  const harness = auth?.attributes.selectedHarness;
  return isWorkspaceHarness(harness) ? harness : undefined;
}

export function isOperatorSessionPrincipal(
  auth:
    | {
        readonly authenticator: string;
        readonly principalId: string;
        readonly principalType: string;
      }
    | null
    | undefined
): boolean {
  return (
    auth?.authenticator === OPERATOR_SESSION_PRINCIPAL.authenticator &&
    auth.principalId === OPERATOR_SESSION_PRINCIPAL.principalId &&
    auth.principalType === OPERATOR_SESSION_PRINCIPAL.principalType
  );
}

export function isOperatorSessionRequest(
  request: OperatorConsoleRequest
): boolean {
  if (request.headers.get(OPERATOR_ACTION_HEADER) !== OPERATOR_SESSION_ACTION) {
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
  // than `request.url`: Vercel routes these paths through a proxy that rewrites
  // the host it dials.
  const origin = request.headers.get("origin");
  if (origin === null) return true;
  const host =
    request.headers.get("x-forwarded-host") ?? request.headers.get("host");
  return host !== null && originHost(origin) === host;
}
