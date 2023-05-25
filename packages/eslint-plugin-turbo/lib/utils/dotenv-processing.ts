import { parse } from "dotenv";
import fs from "fs";
import path from "path";

export function dotEnv(
  workspacePath: string,
  filePaths: string[]
): Set<string> {
  let outputSet = new Set<string>();
  filePaths.forEach((filePath) => {
    var dotEnvFileContents = fs.readFileSync(
      path.join(workspacePath, filePath),
      "utf8"
    );
    Object.keys(parse(dotEnvFileContents)).forEach((envVarName) =>
      outputSet.add(envVarName)
    );
  });

  return outputSet;
}
