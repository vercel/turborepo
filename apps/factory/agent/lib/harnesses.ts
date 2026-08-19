export const HARNESS_IDS = ["claude-code", "codex", "opencode"] as const;
export type HarnessId = (typeof HARNESS_IDS)[number];

export const SANDBOX_IDS = ["vercel"] as const;
export type SandboxId = (typeof SANDBOX_IDS)[number];

export function isHarnessId(value: unknown): value is HarnessId {
  return HARNESS_IDS.some((id) => id === value);
}

export function isSandboxId(value: unknown): value is SandboxId {
  return SANDBOX_IDS.some((id) => id === value);
}
