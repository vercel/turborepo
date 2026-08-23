import { randomUUID } from "node:crypto";

const INTERACTIVE_OIDC_TOKEN_DIRECTORY = "/factory/state";
const FX_TERMINAL_TMUX_SESSION = "factory-fx";

interface SandboxFileWriter {
  readonly writeFiles: (
    files: { content: string | Uint8Array; path: string }[]
  ) => Promise<unknown>;
}

interface SandboxCommandRunner {
  readonly runCommand: (options: {
    readonly args: string[];
    readonly cmd: string;
    readonly cwd: string;
    readonly timeoutMs: number;
  }) => Promise<{
    readonly exitCode: number;
    readonly stdout: () => Promise<string>;
  }>;
}

export interface FxInteractiveLaunch {
  readonly command: "bash";
  readonly args: readonly string[];
}

export async function countFxSessions(
  sandbox: SandboxCommandRunner,
  cwd: string
): Promise<number> {
  const command = await sandbox.runCommand({
    args: ["sessions", "--json", "--limit", "100"],
    cmd: "fx",
    cwd,
    timeoutMs: 10_000
  });
  if (command.exitCode !== 0) return 0;
  try {
    const result = JSON.parse(await command.stdout()) as unknown;
    return typeof result === "object" &&
      result !== null &&
      typeof (result as Record<string, unknown>).count === "number"
      ? (result as Record<string, number>).count
      : 0;
  } catch {
    return 0;
  }
}

export async function prepareFxInteractiveLaunch(
  sandbox: SandboxFileWriter,
  sessionId: string,
  getOidcToken: () => Promise<string>
): Promise<FxInteractiveLaunch> {
  const tokenPath = `${INTERACTIVE_OIDC_TOKEN_DIRECTORY}/interactive-oidc-${randomUUID()}`;
  await sandbox.writeFiles([
    {
      content: Buffer.from(await getOidcToken(), "utf8"),
      path: tokenPath
    }
  ]);
  return {
    command: "bash",
    args: [
      "-lc",
      'token_path="[redacted]"; session_id="$2"; tmux_session="$3"; if tmux has-session -t "$tmux_session" 2>/dev/null; then rm -f "$token_path"; exec tmux attach-session -t "$tmux_session"; fi; launch=\'token_path="$1"; session_id="$2"; trap \'"\'"\'rm -f "$token_path"\'"\'"\' EXIT; export FX_AUTO_UPGRADE=0 FX_PERMISSION_MODE=yolo PATH="/factory/bin:$PATH" VERCEL_OIDC_TOKEN="$(cat "$token_path")"; rm -f "$token_path"; exec fx --record resume --id "$session_id"\'; exec tmux new-session -s "$tmux_session" bash -lc "$launch" factory-terminal "$token_path" "$session_id"',
      "factory-terminal",
      tokenPath,
      sessionId,
      FX_TERMINAL_TMUX_SESSION
    ]
  };
}
