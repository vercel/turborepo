import fs from "fs-extra";
import path from "path";
import glob from "fast-glob";
import yaml from "js-yaml";
import {
  PackageJson,
  PackageManager,
  Project,
  Workspace,
  WorkspaceInfo,
} from "./types";
import { ConvertError } from "./errors";

// adapted from https://github.com/nodejs/corepack/blob/cae770694e62f15fed33dd8023649d77d96023c1/sources/specUtils.ts#L14
const PACKAGE_MANAGER_REGEX = /^(?!_)(.+)@(.+)$/;

function getPackageJson({
  workspaceRoot,
}: {
  workspaceRoot: string;
}): PackageJson {
  const packageJsonPath = path.join(workspaceRoot, "package.json");
  try {
    return fs.readJsonSync(packageJsonPath, "utf8");
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
      const match = packageManager.match(PACKAGE_MANAGER_REGEX);
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
  if (fs.existsSync(workspaceFile)) {
    try {
      const workspaceConfig = yaml.load(fs.readFileSync(workspaceFile, "utf8"));
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
      const { name, description } = getWorkspaceInfo({ workspaceRoot });
      return {
        name,
        description,
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
  getPnpmWorkspaces,
  directoryInfo,
  getMainStep,
};
