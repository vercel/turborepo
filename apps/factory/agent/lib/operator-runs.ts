export const MAINTENANCE_RUN_ACTION = "run-daily-maintenance";
export const PERFORMANCE_RUN_ACTION = "run-daily-performance";

interface OperatorRequest {
  readonly headers: { get(name: string): string | null };
}

export function isOperatorRunRequest(
  request: OperatorRequest,
  action: string
): boolean {
  const origin = request.headers.get("origin");
  const host =
    request.headers.get("x-forwarded-host") ?? request.headers.get("host");
  if (origin === null || host === null) return false;
  try {
    return (
      new URL(origin).host === host &&
      request.headers.get("sec-fetch-site") !== "cross-site" &&
      request.headers.get("x-operator-action") === action &&
      request.headers.get("content-type")?.split(";", 1)[0] ===
        "application/json"
    );
  } catch {
    return false;
  }
}

// Sent as `x-operator-action` by the dashboard and required by the operator
// channel, so both sides of a trigger stay in sync. Ad-hoc chat carries its own
// action on the eve session routes; see `operator-console.ts`.
export type OperatorRunAction =
  | typeof MAINTENANCE_RUN_ACTION
  | typeof PERFORMANCE_RUN_ACTION;
