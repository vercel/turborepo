import path from "node:path";
import { existsSync, writeJSONSync, rmSync, rm } from "fs-extra";
import { ConvertError } from "../errors";
import { updateDependencies } from "../updateDependencies";
import type {
  DetectArgs,
  ReadArgs,
  CreateArgs,
  RemoveArgs,
  ConvertArgs,
  CleanArgs,
  Project,
  ManagerHandler,
  Manager,
} from "../types";
import {
  getMainStep,
  getWorkspaceInfo,
  getPackageJson,
  expandPaths,
  expandWorkspaces,
  getWorkspacePackageManager,
  parseWorkspacePackages,
  isCompatibleWithBunWorkspaces,
  removeLockFile,
} from "../utils";

const PACKAGE_MANAGER_DETAILS: Manager = {
  name: "bun",
  lock: "bun.lockb",
};

/**
 * Check if a given project is using bun workspaces
 * Verify by checking for the existence of:
 *  1. bun.lockb
 *  2. packageManager field in package.json
 */
// eslint-disable-next-line @typescript-eslint/require-await -- must match the detect type signature
async function detect(args: DetectArgs): Promise<boolean> {
  const lockFile = path.join(args.workspaceRoot, PACKAGE_MANAGER_DETAILS.lock);
  const packageManager = getWorkspacePackageManager({
    workspaceRoot: args.workspaceRoot,
  });
  return (
    existsSync(lockFile) || packageManager === PACKAGE_MANAGER_DETAILS.name
  );
}

/**
  Read workspace data from bun workspaces into generic format
*/
async function read(args: ReadArgs): Promise<Project> {
  const isBun = await detect(args);
  if (!isBun) {
    throw new ConvertError("Not a bun project", {
      type: "package_manager-unexpected",
    });
  }

  const packageJson = getPackageJson(args);
  const { name, description } = getWorkspaceInfo(args);
  const workspaceGlobs = parseWorkspacePackages({
    workspaces: packageJson.workspaces,
  });
  return {
    name,
    description,
    packageManager: PACKAGE_MANAGER_DETAILS.name,
    paths: expandPaths({
      root: args.workspaceRoot,
      lockFile: PACKAGE_MANAGER_DETAILS.lock,
    }),
    workspaceData: {
      globs: workspaceGlobs,
      workspaces: expandWorkspaces({
        workspaceGlobs,
        ...args,
      }),
    },
  };
}

/**
 * Create bun workspaces from generic format
 *
 * Creating bun workspaces involves:
 *  1. Validating that the project can be converted to bun workspace
 *  2. Adding the workspaces field in package.json
 *  3. Setting the packageManager field in package.json
 *  4. Updating all workspace package.json dependencies to ensure correct format
 */
// eslint-disable-next-line @typescript-eslint/require-await -- must match the create type signature
async function create(args: CreateArgs): Promise<void> {
  const { project, to, logger, options } = args;
  const hasWorkspaces = project.workspaceData.globs.length > 0;

  if (!isCompatibleWithBunWorkspaces({ project })) {
    throw new ConvertError(
      "Unable to convert project to bun - workspace globs unsupported",
      {
        type: "bun-workspace_glob_error",
      }
    );
  }

  logger.mainStep(
    getMainStep({
      packageManager: PACKAGE_MANAGER_DETAILS.name,
      action: "create",
      project,
    })
  );
  const packageJson = getPackageJson({ workspaceRoot: project.paths.root });
  logger.rootHeader();

  // package manager
  logger.rootStep(
    `adding "packageManager" field to ${path.relative(
      project.paths.root,
      project.paths.packageJson
    )}`
  );
  // TODO: This technically isn't valid as part of the spec (yet)
  packageJson.packageManager = `${to.name}@${to.version}`;

  if (hasWorkspaces) {
    // workspaces field
    logger.rootStep(
      `adding "workspaces" field to ${path.relative(
        project.paths.root,
        project.paths.packageJson
      )}`
    );
    packageJson.workspaces = project.workspaceData.globs;

    if (!options?.dry) {
      writeJSONSync(project.paths.packageJson, packageJson, { spaces: 2 });
    }

    // root dependencies
    updateDependencies({
      workspace: { name: "root", paths: project.paths },
      project,
      to,
      logger,
      options,
    });

    // workspace dependencies
    logger.workspaceHeader();
    project.workspaceData.workspaces.forEach((workspace) => {
      updateDependencies({ workspace, project, to, logger, options });
    });
  } else if (!options?.dry) {
    writeJSONSync(project.paths.packageJson, packageJson, { spaces: 2 });
  }
}

/**
 * Remove bun workspace data
 *
 * Removing bun workspaces involves:
 *  1. Removing the workspaces field from package.json
 *  2. Removing the node_modules directory
 */
async function remove(args: RemoveArgs): Promise<void> {
  const { project, logger, options } = args;
  const hasWorkspaces = project.workspaceData.globs.length > 0;

  logger.mainStep(
    getMainStep({
      packageManager: PACKAGE_MANAGER_DETAILS.name,
      action: "remove",
      project,
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
    `removing "packageManager" field in ${project.name} root "package.json"`
  );
  delete packageJson.packageManager;

  if (!options?.dry) {
    writeJSONSync(project.paths.packageJson, packageJson, { spaces: 2 });

    // collect all workspace node_modules directories
    const allModulesDirs = [
      project.paths.nodeModules,
      ...project.workspaceData.workspaces.map((w) => w.paths.nodeModules),
    ];
    try {
      logger.subStep(`removing "node_modules"`);
      await Promise.all(
        allModulesDirs.map((dir) => rm(dir, { recursive: true, force: true }))
      );
    } catch (err) {
      throw new ConvertError("Failed to remove node_modules", {
        type: "error_removing_node_modules",
      });
    }
  }
}

/**
 * Clean is called post install, and is used to clean up any files
 * from this package manager that were needed for install,
 * but not required after migration
 */
// eslint-disable-next-line @typescript-eslint/require-await -- must match the clean type signature
async function clean(args: CleanArgs): Promise<void> {
  const { project, logger, options } = args;

  logger.subStep(
    `removing ${path.relative(project.paths.root, project.paths.lockfile)}`
  );
  if (!options?.dry) {
    rmSync(project.paths.lockfile, { force: true });
  }
}

/**
 * Attempts to convert an existing, non bun lockfile to a bun lockfile
 *
 * If this is not possible, the non bun lockfile is removed
 */
// eslint-disable-next-line @typescript-eslint/require-await -- must match the convertLock type signature
async function convertLock(args: ConvertArgs): Promise<void> {
  const { project, options } = args;

  // handle moving lockfile from `packageManager` to npm
  switch (project.packageManager) {
    case "pnpm":
      // can't convert from pnpm to bun - just remove the lock
      removeLockFile({ project, options });
      break;
    case "bun":
      // we're already using bun, so we don't need to convert
      break;
    case "npm":
      // can't convert from npm to bun - just remove the lock
      removeLockFile({ project, options });
      break;
    case "yarn":
      // can't convert from yarn to bun - just remove the lock
      removeLockFile({ project, options });
      break;
  }
}

export const bun: ManagerHandler = {
  detect,
  read,
  create,
  remove,
  clean,
  convertLock,
};
