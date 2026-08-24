export const TERM = "xterm-256color";
export const PS1 = `▲ \u0001\u001B[2m\u0002$PWD/\u0001\u001B[0m\u0002 `;
export const DEFAULT_COMMAND = "bash";
export const DEFAULT_ARGS: readonly string[] = ["--noprofile", "--norc", "-i"];
export const DEFAULT_CWD = "/factory/turborepo";

export function buildWebSocketUrl(url: string, token: string): string {
  const separator = url.includes("?") ? "&" : "?";
  return `${url}${separator}token=${encodeURIComponent(token)}`;
}

export function buildStartMessage(cols: number, rows: number) {
  return {
    args: DEFAULT_ARGS,
    cols,
    command: DEFAULT_COMMAND,
    cwd: DEFAULT_CWD,
    env: [`TERM=${TERM}`, `PS1=${PS1}`],
    rows,
    type: "start" as const
  };
}

export function buildResizeMessage(cols: number, rows: number) {
  return { cols, rows, type: "resize" as const };
}

export function parseServerMessage(data: string | Blob | ArrayBuffer):
  | { readonly kind: "output"; readonly data: string }
  | { readonly kind: "exit"; readonly code: number | null }
  | { readonly kind: "unknown" } {
  if (typeof data === "string") {
    try {
      const value: unknown = JSON.parse(data);
      if (
        typeof value === "object" &&
        value !== null &&
        (value as { type?: unknown }).type === "exit"
      ) {
        const code = (value as { code?: unknown }).code;
        if (typeof code === "number" || code === null)
          return { code: code ?? null, kind: "exit" };
      }
    } catch {}
    return { data, kind: "output" };
  }
  if (data instanceof Blob) return { kind: "unknown" };
  return { data: new TextDecoder().decode(data), kind: "output" };
}
