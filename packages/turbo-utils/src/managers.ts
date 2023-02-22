import execa from "execa";
import os from "os";

export type PackageManager = "npm" | "yarn" | "pnpm";
export type PackageManagerAvailable = { available: boolean; version?: string };

async function getVersion(
  packageManager: string
): Promise<PackageManagerAvailable> {
  // run the check from tmpdir to avoid corepack conflicting -
  // this is no longer needed as of https://github.com/nodejs/corepack/pull/167
  // but we'll keep the behavior for those on older versions)
  const execOptions = {
    cwd: os.tmpdir(),
    env: { COREPACK_ENABLE_STRICT: "0" },
  };

  try {
    const result = await execa(packageManager, ["--version"], execOptions);
    return {
      available: true,
      version: result.stdout.trim(),
    };
  } catch (e) {
    return {
      available: false,
    };
  }
}

async function getAvailablePackageManagers(): Promise<
  Record<PackageManager, PackageManagerAvailable>
> {
  const [yarn, pnpm, npm] = await Promise.all([
    getVersion("yarnpkg"),
    getVersion("npm"),
    getVersion("pnpm"),
  ]);

  return {
    yarn,
    pnpm,
    npm,
  };
}

export { getAvailablePackageManagers };
