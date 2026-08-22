export const TERM = "xterm-256color";
export const PS1 = `▲ \u0001\u001B[2m\u0002$PWD/\u0001\u001B[0m\u0002 `;
export const DEFAULT_COMMAND = "sh";
export const DEFAULT_ARGS: readonly string[] = [];
export const DEFAULT_CWD = "/vercel/sandbox";

export interface StartMessage {
  readonly type: "start";
  readonly command: string;
  readonly args: readonly string[];
  readonly env: readonly string[];
  readonly cwd: string;
  readonly cols: number;
  readonly rows: number;
}

export interface ResizeMessage {
  readonly type: "resize";
  readonly cols: number;
  readonly rows: number;
}

export interface ExitMessage {
  readonly type: "exit";
  readonly code: number | null;
}

export type ServerMessage =
  | { readonly kind: "output"; readonly data: string }
  | { readonly kind: "exit"; readonly code: number | null }
  | { readonly kind: "unknown"; readonly data: string };

export function buildWebSocketUrl(url: string, token: string): string {
  const separator = url.includes("?") ? "&" : "?";
  return `${url}${separator}token=${encodeURIComponent(token)}`;
}

export function buildStartMessage(
  cols: number,
  rows: number,
  options: {
    readonly command?: string;
    readonly args?: readonly string[];
    readonly env?: Record<string, string>;
    readonly cwd?: string;
  } = {}
): StartMessage {
  const env = {
    TERM,
    PS1,
    ...options.env
  };

  return {
    type: "start",
    command: options.command ?? DEFAULT_COMMAND,
    args: options.args ?? DEFAULT_ARGS,
    env: Object.entries(env).map(([key, value]) => `${key}=${value}`),
    cwd: options.cwd ?? DEFAULT_CWD,
    cols,
    rows
  };
}

export function buildResizeMessage(cols: number, rows: number): ResizeMessage {
  return { type: "resize", cols, rows };
}

export function parseServerMessage(
  data: string | Blob | ArrayBuffer
): ServerMessage {
  if (typeof data === "string") {
    try {
      const parsed = JSON.parse(data) as unknown;
      if (isExitMessage(parsed)) {
        return { kind: "exit", code: parsed.code };
      }
    } catch {
      // Non-JSON text frame; treat as output.
    }
    return { kind: "output", data };
  }

  if (data instanceof Blob) {
    return { kind: "unknown", data: "[binary blob]" };
  }

  const view = new Uint8Array(data);
  const decoded = new TextDecoder().decode(view);
  return { kind: "output", data: decoded };
}

function isExitMessage(value: unknown): value is ExitMessage {
  return (
    typeof value === "object" &&
    value !== null &&
    (value as Record<string, unknown>).type === "exit" &&
    (typeof (value as Record<string, unknown>).code === "number" ||
      (value as Record<string, unknown>).code === null)
  );
}
