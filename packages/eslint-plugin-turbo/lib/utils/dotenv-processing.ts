import fs from "node:fs";
import path from "node:path";
import { parse } from "dotenv";

interface DotEnvConfig {
  filePaths: Array<string>;
  hashes: Record<string, string | null>;
}

export function dotEnv(
  workspacePath: string | undefined,
  config: DotEnvConfig
): Set<string> {
  if (!workspacePath) {
    return new Set();
  }

  const outputSet = new Set<string>();
  for (const filePath of config.filePaths) {
    try {
      const dotEnvFileContents = fs.readFileSync(
        path.join(workspacePath, filePath),
        "utf8"
      );
      for (const envVarName of Object.keys(parse(dotEnvFileContents))) {
        outputSet.add(envVarName);
      }
    } catch (_) {
      // ignore
    }
  }

  return outputSet;
}
