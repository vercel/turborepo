import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import type { Options } from "execa";
import execa from "execa";
import type { PackageManager } from "./types";

const EXEC_TIMEOUT = 5000;
const SEMVER_VERSION = "\\d+\\.\\d+\\.\\d+(?:-[0-9A-Za-z.-]+)?";
const YARN_PACKAGE_MANAGER_VERSION = new RegExp(
  `^yarn@(?<version>${SEMVER_VERSION})(?:\\+[0-9A-Za-z.-]+)?$`
);
const YARN_RELEASE_PATH_VERSION = new RegExp(
  `^(?:\\./)?\\.yarn/releases/yarn-(?<version>${SEMVER_VERSION})\\.cjs$`
);

interface PackageManagerDetectionOptions {
  projectRoot?: string;
}

interface ProjectYarnMetadata {
  version?: string;
  hasProjectYarnConfig: boolean;
}

async function exec(command: string, args: Array<string> = [], opts?: Options) {
  // run the check from tmpdir to avoid corepack conflicting -
  // this is no longer needed as of https://github.com/nodejs/corepack/pull/167
  // but we'll keep the behavior for those on older versions)
  const execOptions: Options = {
    cwd: os.tmpdir(),
    env: { COREPACK_ENABLE_STRICT: "0" },
    timeout: EXEC_TIMEOUT,
    ...opts
  };
  try {
    const { stdout } = await execa(command, args, execOptions);
    return stdout.trim();
  } catch {
    return undefined;
  }
}

function readFile(filePath: string): string | undefined {
  try {
    return fs.readFileSync(filePath, "utf8");
  } catch {
    return undefined;
  }
}

function readPackageManager(projectRoot: string): string | undefined {
  const packageJson = readFile(path.join(projectRoot, "package.json"));
  if (!packageJson) {
    return undefined;
  }

  try {
    const parsed = JSON.parse(packageJson) as { packageManager?: unknown };
    return typeof parsed.packageManager === "string"
      ? parsed.packageManager
      : undefined;
  } catch {
    return undefined;
  }
}

function parseYarnPath(yarnRc: string): string | undefined {
  const yarnPathLine = yarnRc
    .split(/\r?\n/)
    .find((line) => /^\s*yarnPath\s*:/.test(line));
  if (!yarnPathLine) {
    return undefined;
  }

  const value = yarnPathLine.replace(/^\s*yarnPath\s*:\s*/, "").trim();
  if (!value) {
    return undefined;
  }

  if (value.startsWith('"')) {
    const match = value.match(/^"((?:[^"\\]|\\.)*)"/);
    if (!match) {
      return undefined;
    }
    try {
      return JSON.parse(`"${match[1]}"`) as string;
    } catch {
      return undefined;
    }
  }

  if (value.startsWith("'")) {
    const match = value.match(/^'((?:[^']|'')*)'/);
    return match?.[1].replaceAll("''", "'");
  }

  return value.replace(/\s+#.*$/, "").trim() || undefined;
}

function getYarnReleasePathVersion(yarnPath: string): string | undefined {
  return yarnPath.match(YARN_RELEASE_PATH_VERSION)?.groups?.version;
}

function getYarnReleasePath(version: string): string {
  return `.yarn/releases/yarn-${version}.cjs`;
}

function getProjectYarnMetadata(projectRoot: string): ProjectYarnMetadata {
  const packageManager = readPackageManager(projectRoot);
  const packageManagerVersion = packageManager?.match(
    YARN_PACKAGE_MANAGER_VERSION
  )?.groups?.version;

  if (packageManager?.startsWith("yarn@")) {
    return {
      version: packageManagerVersion,
      hasProjectYarnConfig: true
    };
  }

  const yarnRc = readFile(path.join(projectRoot, ".yarnrc.yml"));
  const yarnPath = yarnRc ? parseYarnPath(yarnRc) : undefined;
  if (!yarnPath) {
    return { hasProjectYarnConfig: false };
  }

  return {
    version: getYarnReleasePathVersion(yarnPath),
    hasProjectYarnConfig: true
  };
}

async function getYarnVersion(projectRoot: string) {
  const metadata = getProjectYarnMetadata(projectRoot);
  if (metadata.hasProjectYarnConfig) {
    return metadata.version;
  }

  return exec("yarnpkg", ["--version"]);
}

async function getYarnBinPath(projectRoot: string) {
  const metadata = getProjectYarnMetadata(projectRoot);
  if (metadata.hasProjectYarnConfig) {
    if (!metadata.version) {
      return undefined;
    }

    if (!metadata.version.startsWith("1.")) {
      return getYarnReleasePath(metadata.version);
    }

    return exec("yarn", ["global", "bin"]);
  }

  const version = await exec("yarnpkg", ["--version"]);
  if (version && !version.startsWith("1.")) {
    return getYarnReleasePath(version);
  } else if (version) {
    return exec("yarn", ["global", "bin"]);
  }
}

export async function getAvailablePackageManagers(
  options: PackageManagerDetectionOptions = {}
): Promise<Record<PackageManager, string | undefined>> {
  const projectRoot = options.projectRoot ?? process.cwd();
  const [yarn, npm, pnpm, bun] = await Promise.all([
    getYarnVersion(projectRoot),
    exec("npm", ["--version"]),
    exec("pnpm", ["--version"]),
    exec("bun", ["--version"])
  ]);

  return {
    yarn,
    pnpm,
    npm,
    bun
  };
}

export async function getPackageManagersBinPaths(
  options: PackageManagerDetectionOptions = {}
): Promise<Record<PackageManager, string | undefined>> {
  const projectRoot = options.projectRoot ?? process.cwd();
  const [yarn, npm, pnpm, bun] = await Promise.all([
    getYarnBinPath(projectRoot),
    exec("npm", ["config", "get", "prefix"]),
    exec("pnpm", ["bin", "--global"]),
    exec("bun", ["pm", "--g", "bin"])
  ]);

  return {
    yarn,
    pnpm,
    npm,
    bun
  };
}
