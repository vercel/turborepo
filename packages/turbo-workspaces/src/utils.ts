import path from "node:path";
import { readJsonSync, existsSync, readFileSync } from "fs-extra";
import { sync as globSync } from "fast-glob";
import yaml from "js-yaml";
import type { PackageJson } from "@turbo/utils";
import type {
  PackageManager,
  Project,
  Workspace,
  WorkspaceInfo,
} from "./types";
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

  if ("packages" in workspaces) {
    return workspaces.packages;
  }

  return workspaces;
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
      const workspacePackageJsonGlob = `${workspaceGlob}/package.json`;
      return globSync(workspacePackageJsonGlob, {
        onlyFiles: true,
        absolute: true,
        cwd: workspaceRoot,
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
};
