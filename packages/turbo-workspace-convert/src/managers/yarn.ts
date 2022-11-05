import fs from "fs-extra";
import path from "path";
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
  expandWorkspaces,
  updateDependencies,
  getWorkspacePackageManager,
} from "../utils";

/*
  check if a given project is using yarn workspaces
*/
async function detect(args: DetectArgs): Promise<boolean> {
  const lockFile = path.join(args.workspaceRoot, "yarn.lock");
  const packageManager = getWorkspacePackageManager({
    workspaceRoot: args.workspaceRoot,
  });
  return fs.existsSync(lockFile) || packageManager === "yarn";
}

/*
 read workspace data from yarn workspaces into generic format
*/
async function read(args: ReadArgs): Promise<Project> {
  const isYarn = await detect(args);
  if (!isYarn) {
    throw new Error("Not a yarn workspaces project");
  }

  const packageJson = getPackageJson(args);
  return {
    name: getWorkspaceName(args),
    packageManager: "yarn",
    paths: {
      root: args.workspaceRoot,
      packageJson: path.join(args.workspaceRoot, "package.json"),
      lockfile: path.join(args.workspaceRoot, "yarn.lock"),
      nodeModules: path.join(args.workspaceRoot, "node_modules"),
    },
    workspaceData: {
      globs: packageJson.workspaces || [],
      workspaces: expandWorkspaces({
        workspaceGlobs: packageJson.workspaces,
        ...args,
      }),
    },
  };
}

/*
 Create yarn workspaces from generic format
 Creating yarn workspaces involves:

  1. Adding the workspaces field in package.json
  2. Setting the packageManager field in package.json
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

  // if we're converting from pnpm, we need to update the workspace package.json files
  if (project.packageManager === "pnpm") {
    // root dependencies
    updateDependencies({
      workspace: { name: "root", paths: project.paths },
      project,
      to,
      logger,
      options,
    });

    logger.workspaceHeader();
    project.workspaceData.workspaces.forEach((workspace) =>
      updateDependencies({ workspace, project, to, logger, options })
    );
  }
}

/*
Remove yarn workspace data

Cleaning up from yarn involves:
  1. Removing the workspaces field from package.json
  2. Removing the yarn.lock file
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
          allModulesDirs.map((dir) => fs.rm(dir, { recursive: true }))
        );
      } catch (err) {
        // only throw here if we find an error other than ENOENT (dir doesn't exist)
        if (
          err &&
          typeof err === "object" &&
          "code" in err &&
          err.code !== "ENOENT"
        ) {
          throw new Error("Failed to remove node_modules");
        }
      }
    }
  } else {
    logger.subStep(`nothing to be done`);
  }
}

/**
 * Clean is called post install, and is used to clean up any files
 * from this package manager that were needed for install
 */
async function clean(args: CleanArgs): Promise<void> {
  const { project, logger, options } = args;

  logger.subStep(
    `removing ${path.relative(project.paths.root, project.paths.lockfile)}`
  );
  if (!options?.dry) {
    fs.removeSync(project.paths.lockfile);
  }
}

/*
converts existing, non yarn lockfile to a yarn lockfile
*/
async function convertLock(args: ConvertArgs): Promise<void> {
  const { project } = args;

  // remove the lockfile
  fs.removeSync(project.paths.lockfile);
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
