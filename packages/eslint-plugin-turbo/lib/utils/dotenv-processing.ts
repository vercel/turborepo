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
  config.filePaths.forEach((filePath) => {
    try {
      const dotEnvFileContents = fs.readFileSync(
        path.join(workspacePath, filePath),
        "utf8"
      );
      Object.keys(parse(dotEnvFileContents)).forEach((envVarName) =>
        outputSet.add(envVarName)
      );
    } catch (_) {
      // ignore
    }
  });

  return outputSet;
}
