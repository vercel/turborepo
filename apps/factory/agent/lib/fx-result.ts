export interface FxTurnResult {
  readonly output: string;
  readonly sessionId: string;
}

export function parseFxTurnResult(
  stdout: string,
  commandExitCode: number
): FxTurnResult | null {
  let value: unknown;
  try {
    value = JSON.parse(stdout);
  } catch {
    return null;
  }
  if (typeof value !== "object" || value === null) return null;
  const result = value as Record<string, unknown>;
  return commandExitCode === 0 &&
    result.exit_code === 0 &&
    typeof result.output === "string" &&
    typeof result.session_id === "string" &&
    result.session_id.length > 0
    ? { output: result.output, sessionId: result.session_id }
    : null;
}
