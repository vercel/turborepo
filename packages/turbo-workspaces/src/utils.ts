import fs from "fs-extra";
import path from "path";
import glob from "fast-glob";
import yaml from "js-yaml";
import semver from "semver";
import { PackageJson, Workspace } from "./types";
import { ConvertError } from "./errors";

// adapted from https://github.com/nodejs/corepack/blob/main/sources/specUtils.ts#L14
const PACKAGE_MANAGER_REGEX = /^(?!_)(.+)@(.+)$/;

function getPackageJson({
  workspaceRoot,
}: {
  workspaceRoot: string;
}): PackageJson {
  const packageJsonPath = path.join(workspaceRoot, "package.json");
  try {
    return fs.readJsonSync(packageJsonPath, "utf8");
  } catch (_) {
    throw new ConvertError(`no "package.json" found at ${workspaceRoot}`);
  }
}

function getWorkspacePackageManager({
  workspaceRoot,
}: {
  workspaceRoot: string;
}): string | undefined {
  const { packageManager } = getPackageJson({ workspaceRoot });
  if (packageManager) {
    const match = packageManager.match(PACKAGE_MANAGER_REGEX);
    if (match && match.length === 3 && semver.valid(match[2])) {
      return match[1];
    }
  }
  return undefined;
}

function getWorkspaceName({
  workspaceRoot,
}: {
  workspaceRoot: string;
}): string {
  const packageJson = getPackageJson({ workspaceRoot });
  if (packageJson.name) {
    return packageJson.name;
  }
  const workspaceDirectory = path.basename(workspaceRoot);
  return workspaceDirectory;
}

function getPnpmWorkspaces({
  workspaceRoot,
}: {
  workspaceRoot: string;
}): Array<string> {
  const workspaceFile = path.join(workspaceRoot, "pnpm-workspace.yaml");
  if (fs.existsSync(workspaceFile)) {
    const workspaceConfig = yaml.load(fs.readFileSync(workspaceFile, "utf8"));
    // validate it's the type we expect
    if (
      workspaceConfig instanceof Object &&
      "packages" in workspaceConfig &&
      Array.isArray(workspaceConfig.packages)
    ) {
      return workspaceConfig.packages as Array<string>;
    }
  }

  return [];
}

function expandWorkspaces({
  workspaceRoot,
  workspaceGlobs,
}: {
  workspaceRoot: string;
  workspaceGlobs?: string[];
}): Array<Workspace> {
  if (!workspaceGlobs) {
    return [];
  }
  return workspaceGlobs
    .flatMap((workspaceGlob) => {
      const workspacePackageJsonGlob = `${workspaceGlob}/package.json`;
      return glob.sync(workspacePackageJsonGlob, {
        onlyFiles: true,
        absolute: true,
        cwd: workspaceRoot,
      });
    })
    .map((workspacePackageJson) => {
      const workspaceRoot = path.dirname(workspacePackageJson);
      const name = getWorkspaceName({ workspaceRoot });
      return {
        name,
        paths: {
          root: workspaceRoot,
          packageJson: workspacePackageJson,
          nodeModules: path.join(workspaceRoot, "node_modules"),
        },
      };
    });
}

function directoryInfo({ directory }: { directory: string }) {
  const dir = path.resolve(process.cwd(), directory);
  return { exists: fs.existsSync(dir), absolute: dir };
}

export {
  getPackageJson,
  getWorkspacePackageManager,
  getWorkspaceName,
  expandWorkspaces,
  getPnpmWorkspaces,
  directoryInfo,
};
