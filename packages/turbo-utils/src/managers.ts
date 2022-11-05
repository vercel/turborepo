import { execSync, ExecSyncOptions } from "child_process";
import os from "os";

export type PackageManagerAvailable = { available: boolean; version?: string };
// run this check from home to avoid corepack conflicting
const execOptions: ExecSyncOptions = { stdio: "pipe", cwd: os.homedir() };

function isNpmAvailable(): PackageManagerAvailable {
  try {
    const result = execSync("npm --version", execOptions);
    return {
      available: true,
      version: result.toString().trim(),
    };
  } catch (e) {
    return {
      available: false,
    };
  }
}

function isPnpmAvailable(): PackageManagerAvailable {
  try {
    const result = execSync("pnpm --version", execOptions);
    return {
      available: true,
      version: result.toString().trim(),
    };
  } catch (e) {
    return {
      available: false,
    };
  }
}

function isYarnAvailable(): PackageManagerAvailable {
  try {
    const result = execSync("yarnpkg --version", execOptions);
    return {
      available: true,
      version: result.toString().trim(),
    };
  } catch (e) {
    return {
      available: false,
    };
  }
}

export { isPnpmAvailable, isYarnAvailable, isNpmAvailable };
