import fs from "fs-extra";
import path from "path";
import { ConvertError } from "../errors";
import updateDependencies from "../updateDependencies";
import {
  DetectArgs,
  ReadArgs,
  CreateArgs,
  RemoveArgs,
  ConvertArgs,
  CleanArgs,
  Project,
} from "../types";
import {
  getWorkspaceName,
  getPackageJson,
  expandPaths,
  expandWorkspaces,
  getWorkspacePackageManager,
} from "../utils";

/**
 * Check if a given project is using yarn workspaces
 * Verify by checking for the existence of:
 *  1. yarn.lock
 *  2. packageManager field in package.json
 */
async function detect(args: DetectArgs): Promise<boolean> {
  const lockFile = path.join(args.workspaceRoot, "yarn.lock");
  const packageManager = getWorkspacePackageManager({
    workspaceRoot: args.workspaceRoot,
  });
  return fs.existsSync(lockFile) || packageManager === "yarn";
}

/**
  Read workspace data from yarn workspaces into generic format
*/
async function read(args: ReadArgs): Promise<Project> {
  const isYarn = await detect(args);
  if (!isYarn) {
    throw new ConvertError("Not a yarn project");
  }

  const packageJson = getPackageJson(args);
  return {
    name: getWorkspaceName(args),
    packageManager: "yarn",
    paths: expandPaths({
      root: args.workspaceRoot,
      lockFile: "yarn.lock",
    }),
    workspaceData: {
      globs: packageJson.workspaces || [],
      workspaces: expandWorkspaces({
        workspaceGlobs: packageJson.workspaces,
        ...args,
      }),
    },
  };
}

/**
 * Create yarn workspaces from generic format
 *
 * Creating yarn workspaces involves:
 *  1. Adding the workspaces field in package.json
 *  2. Setting the packageManager field in package.json
 *  3. Updating all workspace package.json dependencies to ensure correct format
 */
async function create(args: CreateArgs): Promise<void> {
  const { project, to, logger, options } = args;

  logger.mainStep(`Creating yarn workspaces`);
  const packageJson = getPackageJson({ workspaceRoot: project.paths.root });
  logger.rootHeader();
  // workspaces
  if (project.packageManager !== "npm") {
    logger.rootStep(
      `adding "workspaces" field to ${path.relative(
        project.paths.root,
        project.paths.packageJson
      )}`
    );
    packageJson.workspaces = project.workspaceData.globs;
  }

  // package manager
  logger.rootStep(
    `adding "packageManager" field to ${path.relative(
      project.paths.root,
      project.paths.packageJson
    )}`
  );
  packageJson.packageManager = `${to.name}@${to.version}`;

  if (!options?.dry) {
    fs.writeJSONSync(project.paths.packageJson, packageJson, { spaces: 2 });
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
  project.workspaceData.workspaces.forEach((workspace) =>
    updateDependencies({ workspace, project, to, logger, options })
  );
}

/**
 * Remove yarn workspace data
 *
 * Removing yarn workspaces involves:
 *  1. Removing the workspaces field from package.json
 *  2. Removing the node_modules directory
 */
async function remove(args: RemoveArgs): Promise<void> {
  const { project, to, logger, options } = args;

  logger.mainStep(`Removing yarn workspaces`);
  if (to.name !== "npm") {
    const packageJson = getPackageJson({ workspaceRoot: project.paths.root });
    delete packageJson.workspaces;
    logger.subStep(
      `removing "workspaces" field in ${project.name} root "package.json"`
    );

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
        throw new ConvertError("Failed to remove node_modules");
      }
    }
  } else {
    logger.subStep(`nothing to be done`);
  }
}

/**
 * Clean is called post install, and is used to clean up any files
 * from this package manager that were needed for install,
 * but not required after migration
 */
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
 * Attempts to convert an existing, non yarn lockfile to a yarn lockfile
 *
 * If this is not possible, the non yarn lockfile is removed
 */
async function convertLock(args: ConvertArgs): Promise<void> {
  const { project, options } = args;

  // remove the lockfile
  if (!options?.dry) {
    fs.rmSync(project.paths.lockfile, { force: true });
  }
}

const yarn = {
  detect,
  read,
  create,
  remove,
  clean,
  convertLock,
};

export default yarn;
