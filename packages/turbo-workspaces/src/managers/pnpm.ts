import path from "node:path";
import fs from "fs-extra";
import execa from "execa";
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
  expandPaths,
  getWorkspaceInfo,
  expandWorkspaces,
  getPnpmWorkspaces,
  getPackageJson,
  getWorkspacePackageManager,
  removeLockFile,
  bunLockToYarnLock,
} from "../utils";

const PACKAGE_MANAGER_DETAILS: Manager = {
  name: "pnpm",
  lock: "pnpm-lock.yaml",
};

/**
 * Check if a given project is using pnpm workspaces
 * Verify by checking for the existence of:
 *  1. pnpm-workspace.yaml
 *  2. pnpm-workspace.yaml
 */
// eslint-disable-next-line @typescript-eslint/require-await -- must match the detect type signature
async function detect(args: DetectArgs): Promise<boolean> {
  const lockFile = path.join(args.workspaceRoot, PACKAGE_MANAGER_DETAILS.lock);
  const workspaceFile = path.join(args.workspaceRoot, "pnpm-workspace.yaml");
  const packageManager = getWorkspacePackageManager({
    workspaceRoot: args.workspaceRoot,
  });
  return (
    fs.existsSync(lockFile) ||
    fs.existsSync(workspaceFile) ||
    packageManager === PACKAGE_MANAGER_DETAILS.name
  );
}

/**
  Read workspace data from pnpm workspaces into generic format
*/
async function read(args: ReadArgs): Promise<Project> {
  const isPnpm = await detect(args);
  if (!isPnpm) {
    throw new ConvertError("Not a pnpm project", {
      type: "package_manager-unexpected",
    });
  }

  const { name, description } = getWorkspaceInfo(args);
  return {
    name,
    description,
    packageManager: PACKAGE_MANAGER_DETAILS.name,
    paths: expandPaths({
      root: args.workspaceRoot,
      lockFile: PACKAGE_MANAGER_DETAILS.lock,
      workspaceConfig: "pnpm-workspace.yaml",
    }),
    workspaceData: {
      globs: getPnpmWorkspaces(args),
      workspaces: expandWorkspaces({
        workspaceGlobs: getPnpmWorkspaces(args),
        ...args,
      }),
    },
  };
}

/**
 * Create pnpm workspaces from generic format
 *
 * Creating pnpm workspaces involves:
 *  1. Create pnpm-workspace.yaml
 *  2. Setting the packageManager field in package.json
 *  3. Updating all workspace package.json dependencies to ensure correct format
 */
// eslint-disable-next-line @typescript-eslint/require-await -- must match the create type signature
async function create(args: CreateArgs): Promise<void> {
  const { project, to, logger, options } = args;
  const hasWorkspaces = project.workspaceData.globs.length > 0;

  logger.mainStep(
    getMainStep({
      action: "create",
      packageManager: PACKAGE_MANAGER_DETAILS.name,
      project,
    })
  );

  const packageJson = getPackageJson({ workspaceRoot: project.paths.root });
  logger.rootHeader();
  packageJson.packageManager = `${to.name}@${to.version}`;
  logger.rootStep(
    `adding "packageManager" field to ${project.name} root "package.json"`
  );

  // write the changes
  if (!options?.dry) {
    fs.writeJSONSync(project.paths.packageJson, packageJson, { spaces: 2 });

    if (hasWorkspaces) {
      logger.rootStep(`adding "pnpm-workspace.yaml"`);
      fs.writeFileSync(
        path.join(project.paths.root, "pnpm-workspace.yaml"),
        `packages:\n${project.workspaceData.globs
          .map((w) => `  - "${w}"`)
          .join("\n")}`
      );
    }
  }

  if (hasWorkspaces) {
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
  }
}

/**
 * Remove pnpm workspace data
 *
 * Cleaning up from pnpm involves:
 *  1. Removing the pnpm-workspace.yaml file
 *  2. Removing the pnpm-lock.yaml file
 *  3. Removing the node_modules directory
 */
async function remove(args: RemoveArgs): Promise<void> {
  const { project, logger, options } = args;
  const hasWorkspaces = project.workspaceData.globs.length > 0;

  logger.mainStep(
    getMainStep({
      action: "remove",
      packageManager: PACKAGE_MANAGER_DETAILS.name,
      project,
    })
  );
  const packageJson = getPackageJson({ workspaceRoot: project.paths.root });

  if (project.paths.workspaceConfig && hasWorkspaces) {
    logger.subStep(`removing "pnpm-workspace.yaml"`);
    if (!options?.dry) {
      fs.rmSync(project.paths.workspaceConfig, { force: true });
    }
  }

  logger.subStep(
    `removing "packageManager" field in ${project.name} root "package.json"`
  );
  delete packageJson.packageManager;

  if (!options?.dry) {
    fs.writeJSONSync(project.paths.packageJson, packageJson, { spaces: 2 });

    // collect all workspace node_modules directories
    const allModulesDirs = [
      project.paths.nodeModules,
      ...project.workspaceData.workspaces.map((w) => w.paths.nodeModules),
    ];

    try {
      logger.subStep(`removing "node_modules"`);
      await Promise.all(
        allModulesDirs.map((dir) =>
          fs.rm(dir, { recursive: true, force: true })
        )
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
    fs.rmSync(project.paths.lockfile, { force: true });
  }
}

/**
 * Attempts to convert an existing, non pnpm lockfile to a pnpm lockfile
 *
 * If this is not possible, the non pnpm lockfile is removed
 */
async function convertLock(args: ConvertArgs): Promise<void> {
  const { project, options, logger } = args;

  const logLockConversionStep = (): void => {
    logger.subStep(
      `converting ${path.relative(
        project.paths.root,
        project.paths.lockfile
      )} to ${PACKAGE_MANAGER_DETAILS.lock}`
    );
  };

  const importLockfile = async (): Promise<void> => {
    if (!options?.dry && fs.existsSync(project.paths.lockfile)) {
      try {
        await execa(PACKAGE_MANAGER_DETAILS.name, ["import"], {
          stdio: "ignore",
          cwd: project.paths.root,
        });
      } catch (err) {
        // do nothing
      } finally {
        removeLockFile({ project, options });
      }
    }
  };

  // handle moving lockfile from `packageManager` to npm
  switch (project.packageManager) {
    case "pnpm":
      // we're already using pnpm, so we don't need to convert
      break;
    case "bun":
      logLockConversionStep();
      // convert bun -> yarn -> pnpm
      await bunLockToYarnLock({ project, options });
      await importLockfile();
      // remove the intermediate yarn lockfile
      fs.rmSync(path.join(project.paths.root, "yarn.lock"), { force: true });
      break;
    case "npm":
      // convert npm -> pnpm
      logLockConversionStep();
      await importLockfile();
      break;
    case "yarn":
      // convert yarn -> pnpm
      logLockConversionStep();
      await importLockfile();
      break;
  }
}

export const pnpm: ManagerHandler = {
  detect,
  read,
  create,
  remove,
  clean,
  convertLock,
};
