import { parse } from "dotenv";
import fs from "fs";

export function dotEnv(filePaths: string[]): Set<string> {
  let outputSet = new Set<string>();
  filePaths.forEach((filePath) => {
    var dotEnvFileContents = fs.readFileSync(filePath, "utf8");
    Object.keys(parse(dotEnvFileContents)).forEach((envVarName) =>
      outputSet.add(envVarName)
    );
  });

  return outputSet;
}
