import fs from "node:fs";
import path from "node:path";
import type { PackageJson } from "@turbo/utils";
import { error, info } from "./logger";
import type { TurboIgnoreOptions } from "./types";

export function getWorkspace(args: TurboIgnoreOptions): string | null {
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
    const packageJsonContent = JSON.parse(raw) as PackageJson;

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
