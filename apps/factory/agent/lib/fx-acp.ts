export const FX_ACP_SESSION_PATH = "/factory/state/fx-acp-session";
export const FX_ACP_CANCEL_PATH = "/factory/state/fx-acp-cancel";
export const FX_ACP_CLIENT_PATH = "/factory/state/fx-acp-client.mjs";

export const FX_ACP_CLIENT_SOURCE = String.raw`
import { existsSync, writeFileSync } from "node:fs";
import { createInterface } from "node:readline";
import { spawn } from "node:child_process";

const [cwd, prompt, requestedSessionId, sessionPath, cancelPath] = process.argv.slice(2);
const child = spawn("fx", ["acp"], {
  cwd,
  env: process.env,
  stdio: ["pipe", "pipe", "inherit"]
});
const pending = new Map();
let nextId = 1;
let sessionId;
let output = "";
let cancelled = false;

function send(method, params) {
  const id = nextId++;
  child.stdin.write(JSON.stringify({ jsonrpc: "2.0", id, method, params }) + "\n");
  return new Promise((resolve, reject) => pending.set(id, { resolve, reject }));
}

function notify(method, params) {
  child.stdin.write(JSON.stringify({ jsonrpc: "2.0", method, params }) + "\n");
}

const lines = createInterface({ input: child.stdout });
lines.on("line", (line) => {
  let message;
  try { message = JSON.parse(line); } catch { return; }
  if (message.id !== undefined && pending.has(message.id)) {
    const request = pending.get(message.id);
    pending.delete(message.id);
    if (message.error) request.reject(new Error(message.error.message));
    else request.resolve(message.result);
    return;
  }
  const update = message.params?.update;
  if (
    message.method === "session/update" &&
    update?.sessionUpdate === "agent_message_chunk" &&
    update.content?.type === "text"
  ) output += update.content.text;
  if (message.method === "session/request_permission" && message.id !== undefined) {
    const option = message.params?.options?.find(({ kind }) =>
      kind === "allow_once" || kind === "allow_always"
    );
    child.stdin.write(JSON.stringify({
      jsonrpc: "2.0",
      id: message.id,
      result: option
        ? { outcome: { outcome: "selected", optionId: option.optionId } }
        : { outcome: { outcome: "cancelled" } }
    }) + "\n");
  }
});

const cancelTimer = setInterval(() => {
  if (!cancelled && sessionId && existsSync(cancelPath)) {
    cancelled = true;
    notify("session/cancel", { sessionId });
  }
}, 100);

try {
  await send("initialize", {
    protocolVersion: 1,
    clientCapabilities: {},
    clientInfo: { name: "turborepo-factory", version: "1" }
  });
  const session = requestedSessionId
    ? await send("session/load", { cwd, mcpServers: [], sessionId: requestedSessionId })
    : await send("session/new", { cwd, mcpServers: [] });
  sessionId = session.sessionId;
  writeFileSync(sessionPath, sessionId, { mode: 0o600 });
  await send("session/set_mode", { modeId: "code", sessionId }).catch(() => {});
  await send("session/prompt", {
    prompt: [{ type: "text", text: prompt }],
    sessionId
  });
  process.stdout.write(JSON.stringify({ output, sessionId, cancelled }) + "\n");
} finally {
  clearInterval(cancelTimer);
  child.stdin.end();
  child.kill("SIGTERM");
}
`;

export function parseFxAcpResult(stdout: string): {
  readonly cancelled: boolean;
  readonly output: string;
  readonly sessionId: string;
} | null {
  const line = stdout.trim().split("\n").at(-1);
  if (!line) return null;
  try {
    const value = JSON.parse(line) as Record<string, unknown>;
    return typeof value.output === "string" &&
      typeof value.sessionId === "string" &&
      value.sessionId.length > 0 &&
      typeof value.cancelled === "boolean"
      ? {
          cancelled: value.cancelled,
          output: value.output,
          sessionId: value.sessionId
        }
      : null;
  } catch {
    return null;
  }
}
