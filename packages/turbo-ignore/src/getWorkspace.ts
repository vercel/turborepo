import fs from "fs";
import path from "path";
import { error, info } from "./logger";
import { TurboIgnoreArgs } from "./types";

export function getWorkspace({
  args,
}: {
  args: TurboIgnoreArgs;
}): string | null {
  // if the workspace is provided via args, use that
  if (args.workspace) {
    info(`using provided ${args.workspace} as workspace`);
    return args.workspace;
  }

  // otherwise, try and infer it from a package.json in the current directory
  const packageJsonPath = path.join(
    args.directory || process.cwd(),
    "package.json"
  );
  try {
    const raw = fs.readFileSync(packageJsonPath, "utf8");
    const packageJsonContent: Record<string, string> & { name: string } =
      JSON.parse(raw);

    if (!packageJsonContent.name) {
      error(`"${packageJsonPath}" is missing the "name" field (required).`);
      return null;
    }

    info(
      `inferred "${packageJsonContent.name}" as workspace from "package.json"`
    );
    return packageJsonContent.name;
  } catch (e) {
    error(`"${packageJsonPath}" could not be found.`);
    return null;
  }
}
