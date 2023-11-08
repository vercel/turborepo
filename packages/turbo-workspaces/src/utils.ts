import path from "node:path";
import execa from "execa";
import {
  readJsonSync,
  existsSync,
  readFileSync,
  rmSync,
  writeFile,
} from "fs-extra";
import { sync as globSync } from "fast-glob";
import yaml from "js-yaml";
import type { PackageJson, PackageManager } from "@turbo/utils";
import type { Project, Workspace, WorkspaceInfo, Options } from "./types";
import { ConvertError } from "./errors";

// adapted from https://github.com/nodejs/corepack/blob/cae770694e62f15fed33dd8023649d77d96023c1/sources/specUtils.ts#L14
const PACKAGE_MANAGER_REGEX = /^(?!_)(?<manager>.+)@(?<version>.+)$/;

function getPackageJson({
  workspaceRoot,
}: {
  workspaceRoot: string;
}): PackageJson {
  const packageJsonPath = path.join(workspaceRoot, "package.json");
  try {
    return readJsonSync(packageJsonPath, "utf8") as PackageJson;
  } catch (err) {
    if (err && typeof err === "object" && "code" in err) {
      if (err.code === "ENOENT") {
        throw new ConvertError(`no "package.json" found at ${workspaceRoot}`, {
          type: "package_json-missing",
        });
      }
      if (err.code === "EJSONPARSE") {
        throw new ConvertError(
          `failed to parse "package.json" at ${workspaceRoot}`,
          {
            type: "package_json-parse_error",
          }
        );
      }
    }
    throw new Error(
      `unexpected error reading "package.json" at ${workspaceRoot}`
    );
  }
}

function getWorkspacePackageManager({
  workspaceRoot,
}: {
  workspaceRoot: string;
}): string | undefined {
  const { packageManager } = getPackageJson({ workspaceRoot });
  if (packageManager) {
    try {
      const match = PACKAGE_MANAGER_REGEX.exec(packageManager);
      if (match) {
        const [_, manager] = match;
        return manager;
      }
    } catch (err) {
      // this won't always exist.
    }
  }
  return undefined;
}

function getWorkspaceInfo({
  workspaceRoot,
}: {
  workspaceRoot: string;
}): WorkspaceInfo {
  const packageJson = getPackageJson({ workspaceRoot });
  const workspaceDirectory = path.basename(workspaceRoot);

  const { name = workspaceDirectory, description } = packageJson;

  return {
    name,
    description,
  };
}

function getPnpmWorkspaces({
  workspaceRoot,
}: {
  workspaceRoot: string;
}): Array<string> {
  const workspaceFile = path.join(workspaceRoot, "pnpm-workspace.yaml");
  if (existsSync(workspaceFile)) {
    try {
      const workspaceConfig = yaml.load(readFileSync(workspaceFile, "utf8"));
      // validate it's the type we expect
      if (
        workspaceConfig instanceof Object &&
        "packages" in workspaceConfig &&
        Array.isArray(workspaceConfig.packages)
      ) {
        return workspaceConfig.packages as Array<string>;
      }
    } catch (err) {
      throw new ConvertError(`failed to parse ${workspaceFile}`, {
        type: "pnpm-workspace_parse_error",
      });
    }
  }

  return [];
}

function expandPaths({
  root,
  lockFile,
  workspaceConfig,
}: {
  root: string;
  lockFile: string;
  workspaceConfig?: string;
}) {
  const fromRoot = (p: string) => path.join(root, p);
  const paths: Project["paths"] = {
    root,
    lockfile: fromRoot(lockFile),
    packageJson: fromRoot("package.json"),
    nodeModules: fromRoot("node_modules"),
  };

  if (workspaceConfig) {
    paths.workspaceConfig = fromRoot(workspaceConfig);
  }

  return paths;
}

function parseWorkspacePackages({
  workspaces,
}: {
  workspaces: PackageJson["workspaces"];
}): Array<string> {
  if (!workspaces) {
    return [];
  }

  if (Array.isArray(workspaces)) {
    return workspaces;
  }

  if ("packages" in workspaces) {
    return workspaces.packages ?? [];
  }

  return [];
}

function expandWorkspaces({
  workspaceRoot,
  workspaceGlobs,
}: {
  workspaceRoot: string;
  workspaceGlobs?: Array<string>;
}): Array<Workspace> {
  if (!workspaceGlobs) {
    return [];
  }
  return workspaceGlobs
    .flatMap((workspaceGlob) => {
      const workspacePackageJsonGlob = [`${workspaceGlob}/package.json`];
      return globSync(workspacePackageJsonGlob, {
        onlyFiles: true,
        absolute: true,
        cwd: workspaceRoot,
        ignore: ["**/node_modules/**"],
      });
    })
    .map((workspacePackageJson) => {
      const root = path.dirname(workspacePackageJson);
      const { name, description } = getWorkspaceInfo({ workspaceRoot: root });
      return {
        name,
        description,
        paths: {
          root,
          packageJson: workspacePackageJson,
          nodeModules: path.join(root, "node_modules"),
        },
      };
    });
}

function directoryInfo({ directory }: { directory: string }) {
  const dir = path.resolve(process.cwd(), directory);
  return { exists: existsSync(dir), absolute: dir };
}

function getMainStep({
  packageManager,
  action,
  project,
}: {
  packageManager: PackageManager;
  action: "create" | "remove";
  project: Project;
}) {
  const hasWorkspaces = project.workspaceData.globs.length > 0;
  return `${action === "remove" ? "Removing" : "Adding"} ${packageManager} ${
    hasWorkspaces ? "workspaces" : ""
  } ${action === "remove" ? "from" : "to"} ${project.name}`;
}

/**
 * At the time of writing, bun only support simple globs (can only end in /*) for workspaces. This means we can't convert all projects
 * from other package manager workspaces to bun workspaces, we first have to validate that the globs are compatible.
 *
 * NOTE: It's possible a project could work with bun workspaces, but just not in the way its globs are currently defined. We will
 * not change existing globs to make a project work with bun, we will only convert projects that are already compatible.
 *
 * This function matches the behavior of bun's glob validation: https://github.com/oven-sh/bun/blob/92e95c86dd100f167fb4cf8da1db202b5211d2c1/src/install/lockfile.zig#L2889
 */
function isCompatibleWithBunWorkspaces({
  project,
}: {
  project: Project;
}): boolean {
  const validator = (glob: string) => {
    if (glob.includes("*")) {
      // no multi level globs
      if (glob.includes("**")) {
        return false;
      }

      // no * in the middle of a path
      const withoutLastPathSegment = glob.split("/").slice(0, -1).join("/");
      if (withoutLastPathSegment.includes("*")) {
        return false;
      }
    }
    // no fancy glob patterns
    if (["!", "[", "]", "{", "}"].some((char) => glob.includes(char))) {
      return false;
    }

    return true;
  };

  return project.workspaceData.globs.every(validator);
}

function removeLockFile({
  project,
  options,
}: {
  project: Project;
  options?: Options;
}) {
  if (!options?.dry) {
    // remove the lockfile
    rmSync(project.paths.lockfile, { force: true });
  }
}

async function bunLockToYarnLock({
  project,
  options,
}: {
  project: Project;
  options?: Options;
}) {
  if (!options?.dry && existsSync(project.paths.lockfile)) {
    try {
      const { stdout } = await execa("bun", ["bun.lockb"], {
        stdin: "ignore",
        cwd: project.paths.root,
      });
      // write the yarn lockfile
      await writeFile(path.join(project.paths.root, "yarn.lock"), stdout);
    } catch (err) {
      // do nothing
    } finally {
      // remove the old lockfile
      rmSync(project.paths.lockfile, { force: true });
    }
  }
}

export {
  getPackageJson,
  getWorkspacePackageManager,
  getWorkspaceInfo,
  expandPaths,
  expandWorkspaces,
  parseWorkspacePackages,
  getPnpmWorkspaces,
  directoryInfo,
  getMainStep,
  isCompatibleWithBunWorkspaces,
  removeLockFile,
  bunLockToYarnLock,
};
