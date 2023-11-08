import type { ExecSyncOptions } from "node:child_process";
import { execSync } from "node:child_process";

function exec(
  command: string,
  opts: ExecSyncOptions,
  fallback?: string
): string | undefined {
  try {
    const rawResult = execSync(command, { stdio: "pipe", ...opts });
    return rawResult.toString("utf8").trim();
  } catch (err) {
    return fallback || undefined;
  }
}

export { exec };
