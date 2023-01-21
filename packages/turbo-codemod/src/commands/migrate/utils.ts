import { execSync, ExecSyncOptions } from "child_process";

function exec(
  command: string,
  opts: ExecSyncOptions,
  fallback?: string
): string | undefined {
  try {
    const rawResult = execSync(command, opts);
    return rawResult.toString("utf8").trim();
  } catch (err) {
    return fallback || undefined;
  }
}

export { exec };
