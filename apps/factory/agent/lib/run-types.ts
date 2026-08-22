import type { HarnessId, SandboxId } from "./harnesses";

export type AgentRunStatus = "running" | "waiting" | "completed" | "failed";
export type SandboxRunStatus =
  | "provisioning"
  | "running"
  | "stopped"
  | "failed";

export interface AgentRunRecord {
  readonly agent: string;
  readonly finishedAt?: string;
  readonly harness?: HarnessId;
  readonly id: string;
  readonly model?: string;
  readonly sandbox?: {
    readonly id: string;
    readonly provider: SandboxId | "eve";
    readonly status: SandboxRunStatus;
  };
  readonly source: "eve" | "harness";
  readonly startedAt: string;
  readonly status: AgentRunStatus;
  readonly title: string;
  readonly trigger: string;
  readonly updatedAt: string;
}

export function isAgentRunRecord(value: unknown): value is AgentRunRecord {
  if (typeof value !== "object" || value === null) return false;
  const run = value as Record<string, unknown>;
  return (
    typeof run.agent === "string" &&
    typeof run.id === "string" &&
    (run.source === "eve" || run.source === "harness") &&
    typeof run.startedAt === "string" &&
    (run.status === "running" ||
      run.status === "waiting" ||
      run.status === "completed" ||
      run.status === "failed") &&
    typeof run.title === "string" &&
    typeof run.trigger === "string" &&
    typeof run.updatedAt === "string"
  );
}
