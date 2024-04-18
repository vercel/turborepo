import os from "node:os";
import crypto from "node:crypto";
import path from "node:path";
import { configDir } from "dirs-next";
import type { PackageInfo } from "./events/types";

function buildUserAgent(packageInfo: PackageInfo): string {
  const nodeVersion = process.version;
  const operatingSystem = os.type();
  const architecture = os.arch();

  return `${packageInfo.name} ${packageInfo.version} ${nodeVersion} ${operatingSystem} ${architecture}`;
}

async function defaultConfigPath() {
  const dir = process.env.TURBO_CONFIG_DIR_PATH
    ? process.env.TURBO_CONFIG_DIR_PATH
    : await configDir();

  if (!dir) {
    throw new Error("Could not find telemetry config directory");
  }

  return path.join(dir, "turborepo", "telemetry.json");
}

function oneWayHashWithSalt({ input, salt }: { input: string; salt: string }) {
  return crypto.createHash("sha256").update(`${salt}${input}`).digest("hex");
}

// Exported asn an object instead of export keyword, so that these functions
// can be mocked in tests.
export default {
  buildUserAgent,
  oneWayHashWithSalt,
  defaultConfigPath,
};
