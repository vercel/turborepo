import os from "node:os";
import path from "node:path";
import { configDir } from "dirs-next";
import type { PackageInfo } from "./client";

export function buildUserAgent(packageInfo: PackageInfo): string {
  const nodeVersion = process.version;
  const operatingSystem = os.type();
  const architecture = os.arch();

  return `${packageInfo.name} ${packageInfo.version} ${nodeVersion} ${operatingSystem} ${architecture}`;
}

export async function defaultConfigPath() {
  const dir = await configDir();
  if (!dir) {
    throw new Error("Could not find telemetry config directory");
  }

  return path.join(dir, "turborepo", "telemetry.json");
}
