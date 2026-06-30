import path from "node:path";
import fs from "fs-extra";
import { ConvertError } from "../errors";
import { updateDependencies } from "../update-dependencies";
import type {
  DetectArgs,
  ReadArgs,
  CreateArgs,
  RemoveArgs,
  ConvertArgs,
  CleanArgs,
  Project,
  ManagerHandler,
  Manager
} from "../types";
import {
  getMainStep,
  getWorkspaceInfo,
  getPackageJson,
  expandPaths,
  expandWorkspaces,
  getWorkspacePackageManager,
  setPackageManagerDeclaration,
  removePackageManagerDeclaration,
  parseWorkspacePackages,
  getPnpmWorkspaces,
  getUnderlyingLockfileManager,
  getUnderlyingLockfileName,
  removeLockFile
} from "../utils";
import { npm } from "./npm";
import { pnpm } from "./pnpm";
import { yarn } from "./yarn";
import { bun } from "./bun";

const PACKAGE_MANAGER_DETAILS: Manager = {
  name: "nub",
  lock: "package-lock.json"
};

const UNDERLYING_MANAGERS = {
  npm,
  pnpm,
  yarn,
  bun
} as const;

/**
 * nub is recognized only through the `packageManager` field in `package.json`
 * (`"nub@x.y.z"`). A foreign lockfile alone does not imply nub.
 */
// eslint-disable-next-line @typescript-eslint/require-await -- must match the detect type signature
async function detect(args: DetectArgs): Promise<boolean> {
  const packageManager = getWorkspacePackageManager({
    workspaceRoot: args.workspaceRoot
  });
  return packageManager === PACKAGE_MANAGER_DETAILS.name;
}

async function read(args: ReadArgs): Promise<Project> {
  const isNub = await detect(args);
  if (!isNub) {
    throw new ConvertError("Not a nub project", {
      type: "package_manager-unexpected"
    });
  }

  const underlying = getUnderlyingLockfileManager({
    workspaceRoot: args.workspaceRoot
  });
  const underlyingHandler = UNDERLYING_MANAGERS[underlying];

  if (await underlyingHandler.detect(args)) {
    const project = await underlyingHandler.read(args);
    return { ...project, packageManager: PACKAGE_MANAGER_DETAILS.name };
  }

  const packageJson = getPackageJson(args);
  const { name, description } = getWorkspaceInfo(args);
  const lockfile = getUnderlyingLockfileName({
    workspaceRoot: args.workspaceRoot
  });
  const pnpmGlobs = getPnpmWorkspaces(args);
  const workspaceGlobs =
    underlying === "pnpm" && pnpmGlobs.length > 0
      ? pnpmGlobs
      : parseWorkspacePackages({ workspaces: packageJson.workspaces });

  return {
    name,
    description,
    packageManager: PACKAGE_MANAGER_DETAILS.name,
    paths: expandPaths({
      root: args.workspaceRoot,
      lockFile: lockfile,
      workspaceConfig:
        underlying === "pnpm" && pnpmGlobs.length > 0
          ? "pnpm-workspace.yaml"
          : undefined
    }),
    workspaceData: {
      globs: workspaceGlobs,
      workspaces: expandWorkspaces({
        workspaceGlobs,
        ...args
      })
    }
  };
}

// eslint-disable-next-line @typescript-eslint/require-await -- must match the create type signature
async function create(args: CreateArgs): Promise<void> {
  const { project, options, to, logger } = args;
  const hasWorkspaces = project.workspaceData.globs.length > 0;

  logger.mainStep(
    getMainStep({
      packageManager: PACKAGE_MANAGER_DETAILS.name,
      action: "create",
      project
    })
  );
  const packageJson = getPackageJson({ workspaceRoot: project.paths.root });
  logger.rootHeader();

  logger.rootStep(
    `adding "devEngines.packageManager" field to ${path.relative(
      project.paths.root,
      project.paths.packageJson
    )}`
  );
  setPackageManagerDeclaration({
    packageJson,
    packageManager: to.name,
    version: to.version
  });

  if (hasWorkspaces) {
    logger.rootStep(
      `adding "workspaces" field to ${path.relative(
        project.paths.root,
        project.paths.packageJson
      )}`
    );
    packageJson.workspaces = project.workspaceData.globs;

    if (!options?.dry) {
      fs.writeJSONSync(project.paths.packageJson, packageJson, { spaces: 2 });
    }

    updateDependencies({
      workspace: { name: "root", paths: project.paths },
      project,
      to,
      logger,
      options
    });

    logger.workspaceHeader();
    for (const workspace of project.workspaceData.workspaces) {
      updateDependencies({ workspace, project, to, logger, options });
    }
  } else if (!options?.dry) {
    fs.writeJSONSync(project.paths.packageJson, packageJson, { spaces: 2 });
  }
}

async function remove(args: RemoveArgs): Promise<void> {
  const { project, logger, options } = args;
  const hasWorkspaces = project.workspaceData.globs.length > 0;

  logger.mainStep(
    getMainStep({
      packageManager: PACKAGE_MANAGER_DETAILS.name,
      action: "remove",
      project
    })
  );
  const packageJson = getPackageJson({ workspaceRoot: project.paths.root });

  if (hasWorkspaces) {
    logger.subStep(
      `removing "workspaces" field in ${project.name} root "package.json"`
    );
    delete packageJson.workspaces;
  }

  logger.subStep(
    `removing ${PACKAGE_MANAGER_DETAILS.name} package manager declarations in ${project.name} root "package.json"`
  );
  removePackageManagerDeclaration({
    packageJson,
    packageManager: PACKAGE_MANAGER_DETAILS.name
  });

  if (!options?.dry) {
    fs.writeJSONSync(project.paths.packageJson, packageJson, { spaces: 2 });

    const allModulesDirs = [
      project.paths.nodeModules,
      ...project.workspaceData.workspaces.map((w) => w.paths.nodeModules)
    ];
    try {
      logger.subStep(`removing "node_modules"`);
      await Promise.all(
        allModulesDirs.map((dir) =>
          fs.rm(dir, { recursive: true, force: true })
        )
      );
    } catch {
      throw new ConvertError("Failed to remove node_modules", {
        type: "error_removing_node_modules"
      });
    }
  }
}

// eslint-disable-next-line @typescript-eslint/require-await -- must match the clean type signature
async function clean(args: CleanArgs): Promise<void> {
  const { project, logger, options } = args;

  logger.subStep(
    `removing ${path.relative(project.paths.root, project.paths.lockfile)}`
  );
  if (!options?.dry) {
    fs.rmSync(project.paths.lockfile, { force: true });
  }
}

// eslint-disable-next-line @typescript-eslint/require-await -- must match the convertLock type signature
async function convertLock(args: ConvertArgs): Promise<void> {
  const { project, options } = args;

  switch (project.packageManager) {
    case "pnpm":
    case "bun":
    case "yarn": {
      removeLockFile({ project, options });
      break;
    }
    case "npm":
    case "nub": {
      break;
    }
  }
}

export const nub: ManagerHandler = {
  detect,
  read,
  create,
  remove,
  clean,
  convertLock
};
