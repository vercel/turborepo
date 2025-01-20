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
    // Yarn berry doesn't have a global bin so this checks from the repo root
    // If the repo uses berry, it will return it's specified version
    exec("yarnpkg", ["--version"], { cwd: "." }),
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
    // yarn berry doesn't have a global bin so we check from within the repo
    exec("yarnpkg", ["--version"], { cwd: "." }).then((version) => {
      if (version && !version.startsWith("1.")) {
        return `.yarn/releases/yarn-${version}.cjs`;
        // yarn 1
      } else if (version) {
        return exec("yarn", ["global", "bin"]);
      }
    }),
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
