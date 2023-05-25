import { parse } from "dotenv";
import fs from "fs";
import path from "path";

type DotEnvConfig = {
  filePaths: string[];
  hashes: {
    [path: string]: string | null;
  };
};

export function dotEnv(
  workspacePath: string | undefined,
  config: DotEnvConfig
): Set<string> {
  if (!workspacePath) {
    return new Set();
  }

  let outputSet = new Set<string>();
  config.filePaths.forEach((filePath) => {
    try {
      var dotEnvFileContents = fs.readFileSync(
        path.join(workspacePath, filePath),
        "utf8"
      );
      Object.keys(parse(dotEnvFileContents)).forEach((envVarName) =>
        outputSet.add(envVarName)
      );
    } catch (e) {}
  });

  return outputSet;
}
