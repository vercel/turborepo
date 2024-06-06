import fs from "node:fs";
import path from "node:path";
import type { PackageJson } from "@turbo/utils";
import { parse as JSON5Parse } from "json5";
import { error, info } from "./logger";
import type { TurboIgnoreOptions } from "./types";

export function getTurboVersion(
  args: TurboIgnoreOptions,
  root: string
): string | null {
  let { turboVersion } = args;
  if (turboVersion) {
    info(`Using turbo version "${turboVersion}" from arguments`);
    return turboVersion;
  }

  const packageJsonPath = path.join(root, "package.json");
  try {
    const raw = fs.readFileSync(packageJsonPath, "utf8");
    const packageJson = JSON.parse(raw) as PackageJson;
    const dependencies = packageJson.dependencies?.turbo;
    const devDependencies = packageJson.devDependencies?.turbo;
    turboVersion = dependencies || devDependencies;
    if (turboVersion !== undefined) {
      info(`Inferred turbo version "${turboVersion}" from "package.json"`);
      return turboVersion;
    }
  } catch (e) {
    error(
      `"${packageJsonPath}" could not be read. turbo-ignore turbo version inference failed`
    );
    return null;
  }

  const turboJSONPath = path.join(root, "turbo.json");
  try {
    const rawTurboJson = fs.readFileSync(turboJSONPath, "utf8");
    const turboJson: { tasks?: unknown; pipeline?: unknown } =
      JSON5Parse(rawTurboJson);
    if ("tasks" in turboJson) {
      info(`Inferred turbo version ^2 based on "tasks" in "turbo.json"`);
      return "^2";
    }
    if ("pipeline" in turboJson) {
      info(`Inferred turbo version ^1 based on "pipeline" in "turbo.json"`);
      return "^1";
    }
    return null;
  } catch (e) {
    error(
      `"${turboJSONPath}" could not be read. turbo-ignore turbo version inference failed`
    );
    return null;
  }
}
