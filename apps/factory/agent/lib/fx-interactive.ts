import { randomUUID } from "node:crypto";

const INTERACTIVE_OIDC_TOKEN_DIRECTORY = "/factory/state";
const FX_ACP_CANCEL_PATH = "/factory/state/fx-acp-cancel";

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

export async function cancelFxAcpTurn(sandbox: SandboxFileWriter): Promise<void> {
  await sandbox.writeFiles([
    { content: Buffer.from("cancel\n", "utf8"), path: FX_ACP_CANCEL_PATH }
  ]);
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
      'token_path="$1"; session_id="$2"; trap \'rm -f "$token_path"\' EXIT; export VERCEL_OIDC_TOKEN="$(cat "$token_path")"; rm -f "$token_path"; exec fx --yolo --resume "$session_id"',
      "factory-terminal",
      tokenPath,
      sessionId
    ]
  };
}
