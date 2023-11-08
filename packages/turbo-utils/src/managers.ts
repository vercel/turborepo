import os from "node:os";
import type { Options } from "execa";
import execa from "execa";
import type { PackageManager } from "./types";

async function exec(command: string, args: Array<string> = [], opts?: Options) {
  // run the check from tmpdir to avoid corepack conflicting -
  // this is no longer needed as of https://github.com/nodejs/corepack/pull/167
  // but we'll keep the behavior for those on older versions)
  const execOptions: Options = {
    cwd: os.tmpdir(),
    env: { COREPACK_ENABLE_STRICT: "0" },
    ...opts,
  };
  try {
    const { stdout } = await execa(command, args, execOptions);
    return stdout.trim();
  } catch {
    return undefined;
  }
}

export async function getAvailablePackageManagers(): Promise<
  Record<PackageManager, string | undefined>
> {
  const [yarn, npm, pnpm, bun] = await Promise.all([
    exec("yarnpkg", ["--version"]),
    exec("npm", ["--version"]),
    exec("pnpm", ["--version"]),
    exec("bun", ["--version"]),
  ]);

  return {
    yarn,
    pnpm,
    npm,
    bun,
  };
}

export async function getPackageManagersBinPaths(): Promise<
  Record<PackageManager, string | undefined>
> {
  const [yarn, npm, pnpm, bun] = await Promise.all([
    exec("yarnpkg", ["global", "bin"]),
    exec("npm", ["config", "get", "prefix"]),
    exec("pnpm", ["bin", "--global"]),
    exec("bun", ["pm", "--g", "bin"]),
  ]);

  return {
    yarn,
    pnpm,
    npm,
    bun,
  };
}
