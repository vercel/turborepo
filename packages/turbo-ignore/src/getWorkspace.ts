import fs from "fs";
import path from "path";
import { error, info } from "./logger";
import { TurboIgnoreArgs } from "./types";

export function getWorkspace(args: TurboIgnoreArgs): string | null {
  const { directory = process.cwd(), workspace } = args;

  // if the workspace is provided via args, use that
  if (workspace) {
    info(`Using "${workspace}" as workspace from arguments`);
    return workspace;
  }

  // otherwise, try and infer it from a package.json in the current directory
  const packageJsonPath = path.join(directory, "package.json");
  try {
    const raw = fs.readFileSync(packageJsonPath, "utf8");
    const packageJsonContent: Record<string, string> & { name: string } =
      JSON.parse(raw);

    if (!packageJsonContent.name) {
      error(`"${packageJsonPath}" is missing the "name" field (required).`);
      return null;
    }

    info(
      `Inferred "${packageJsonContent.name}" as workspace from "package.json"`
    );
    return packageJsonContent.name;
  } catch (e) {
    error(
      `"${packageJsonPath}" could not be found. turbo-ignore inferencing failed`
    );
    return null;
  }
}
