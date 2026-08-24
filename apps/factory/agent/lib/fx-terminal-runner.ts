export const FX_TERMINAL_SESSION_PATH = "/factory/state/fx-terminal-session";
export const FX_TERMINAL_RUNNER_PATH = "/factory/state/fx-terminal-runner.mjs";
export const FX_TERMINAL_TMUX_SESSION = "factory-fx";

export const FX_TERMINAL_RUNNER_SOURCE = String.raw`
import { readFileSync, writeFileSync } from "node:fs";
import { spawnSync } from "node:child_process";

const [cwd, promptPath, tokenPath, requestedSessionId, model, sessionPath, tmuxSession] =
  process.argv.slice(2);

function run(command, args, options = {}) {
  const result = spawnSync(command, args, {
    cwd,
    encoding: "utf8",
    env: process.env,
    ...options
  });
  if (result.status !== 0 && !options.allowFailure) {
    throw new Error(result.stderr || result.stdout || command + " failed");
  }
  return result;
}

function inspectSession(sessionId) {
  const result = run("fx", ["session", "--id", sessionId, "--json"], {
    allowFailure: true
  });
  if (result.status !== 0) return null;
  try { return JSON.parse(result.stdout); } catch { return null; }
}

function latestSessionId() {
  const result = run("fx", ["sessions", "--json", "--limit", "1"], {
    allowFailure: true
  });
  if (result.status !== 0) return undefined;
  try { return JSON.parse(result.stdout).sessions?.[0]?.id; } catch { return undefined; }
}

const baseline = requestedSessionId
  ? inspectSession(requestedSessionId)?.history_len ?? 0
  : 0;
const previousSessionId = requestedSessionId ? undefined : latestSessionId();
const launch =
  'token_path="$1"; session_id="$2"; ' +
  'trap \'rm -f "$token_path"\' EXIT; ' +
  'export FX_AUTO_UPGRADE=0 FX_PERMISSION_MODE=yolo PATH="/factory/bin:$PATH"; ' +
  'if [ -n "$3" ]; then export FX_MODEL="$3"; fi; ' +
  'export VERCEL_OIDC_TOKEN="$(cat "$token_path")"; rm -f "$token_path"; ' +
  'if [ -n "$session_id" ]; then exec fx --record resume --id "$session_id"; ' +
  'else exec fx --record; fi';
const paneArgs = [
  "-c", cwd,
  "bash", "-lc", launch, "factory-terminal", tokenPath, requestedSessionId, model
];

const exists = run("tmux", ["has-session", "-t", tmuxSession], {
  allowFailure: true
}).status === 0;
run("tmux", exists
  ? ["respawn-pane", "-k", "-t", tmuxSession, ...paneArgs]
  : ["new-session", "-d", "-s", tmuxSession, ...paneArgs]);

let sessionId = requestedSessionId;
for (let attempt = 0; !sessionId && attempt < 100; attempt += 1) {
  Atomics.wait(new Int32Array(new SharedArrayBuffer(4)), 0, 0, 100);
  const latest = latestSessionId();
  if (latest && latest !== previousSessionId) sessionId = latest;
}
if (!sessionId) throw new Error("fx did not create an interactive session.");
writeFileSync(sessionPath, sessionId, { mode: 0o600 });

Atomics.wait(new Int32Array(new SharedArrayBuffer(4)), 0, 0, 300);
run("tmux", ["load-buffer", "-b", "factory-prompt", promptPath]);
run("tmux", ["paste-buffer", "-b", "factory-prompt", "-t", tmuxSession, "-d"]);
run("tmux", ["send-keys", "-t", tmuxSession, "Enter"]);

for (;;) {
  Atomics.wait(new Int32Array(new SharedArrayBuffer(4)), 0, 0, 1000);
  const session = inspectSession(sessionId);
  if (!session || session.history_len <= baseline) continue;
  const turn = session.history?.at(-1);
  if (turn?.kind !== "assistant" || typeof turn.assistant !== "string") continue;
  process.stdout.write(JSON.stringify({ output: turn.assistant, sessionId }) + "\n");
  break;
}
`;

export function parseFxTerminalResult(stdout: string): {
  readonly output: string;
  readonly sessionId: string;
} | null {
  const line = stdout.trim().split("\n").at(-1);
  if (!line) return null;
  try {
    const value = JSON.parse(line) as Record<string, unknown>;
    return typeof value.output === "string" &&
      typeof value.sessionId === "string" &&
      value.sessionId.length > 0
      ? { output: value.output, sessionId: value.sessionId }
      : null;
  } catch {
    return null;
  }
}
