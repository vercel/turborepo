import { createHmac, timingSafeEqual } from "node:crypto";

export const MAINTENANCE_RUN_ACTION = "run-daily-maintenance";
export const PERFORMANCE_RUN_ACTION = "run-daily-performance";

// Sent as `x-operator-action` by the dashboard and required by the operator
// channel, so both sides of a trigger stay in sync.
export type OperatorRunAction =
  | typeof MAINTENANCE_RUN_ACTION
  | typeof PERFORMANCE_RUN_ACTION;

interface WorkflowRunCapability {
  readonly exp: number;
  readonly sessionID: string;
  readonly workflowRunID: string;
}

export function signWorkflowRun(
  value: Omit<WorkflowRunCapability, "exp">,
  secret: string
): string {
  const payload = Buffer.from(JSON.stringify({
    ...value,
    exp: Math.floor(Date.now() / 1000) + 24 * 60 * 60
  })).toString("base64url");
  const signature = createHmac("sha256", secret).update(payload).digest("base64url");
  return `${payload}.${signature}`;
}

export function verifyWorkflowRun(
  token: string,
  sessionID: string,
  secret: string
): string | null {
  const [payload, signature] = token.split(".");
  if (!payload || !signature) return null;
  const expected = createHmac("sha256", secret).update(payload).digest("base64url");
  const left = Buffer.from(signature);
  const right = Buffer.from(expected);
  if (left.length !== right.length || !timingSafeEqual(left, right)) return null;
  try {
    const value = JSON.parse(Buffer.from(payload, "base64url").toString()) as WorkflowRunCapability;
    return value.sessionID === sessionID &&
      value.exp > Date.now() / 1000 &&
      typeof value.workflowRunID === "string"
      ? value.workflowRunID
      : null;
  } catch {
    return null;
  }
}
