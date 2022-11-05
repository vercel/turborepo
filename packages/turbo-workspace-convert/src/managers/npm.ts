import fs from "fs-extra";
import path from "path";
import {
  DetectArgs,
  ReadArgs,
  CreateArgs,
  RemoveArgs,
  CleanArgs,
  Project,
  ConvertArgs,
  ManagerHandler,
} from "../types";
import {
  getWorkspaceName,
  getPackageJson,
  expandWorkspaces,
  updateDependencies,
  getWorkspacePackageManager,
} from "../utils";

/*
 Check if a given project using npm workspaces

 Verify by checking for the existence of:
  1. package-lock.json
  2.
*/
async function detect(args: DetectArgs): Promise<boolean> {
  const lockFile = path.join(args.workspaceRoot, "package-lock.json");
  const packageManager = getWorkspacePackageManager({
    workspaceRoot: args.workspaceRoot,
  });
  return fs.existsSync(lockFile) || packageManager === "npm";
}

/*
 Read workspace data from npm workspaces into generic format
*/
async function read(args: ReadArgs): Promise<Project> {
  const isNpm = await detect(args);
  if (!isNpm) {
    throw new Error("Not an npm workspaces project");
  }

  const packageJson = getPackageJson(args);
  return {
    name: getWorkspaceName(args),
    packageManager: "npm",
    paths: {
      root: args.workspaceRoot,
      packageJson: path.join(args.workspaceRoot, "package.json"),
      lockfile: path.join(args.workspaceRoot, "package-lock.json"),
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
 Create npm workspaces from generic format
 Creating npm workspaces involves:

  1. Adding the workspaces field in package.json
  2. Setting the packageManager field in package.json
  3. Updating all workspace package.json dependencies to ensure correct format
*/
async function create(args: CreateArgs): Promise<void> {
  const { project, options, to, logger } = args;

  logger.mainStep(`Creating npm workspaces`);
  const packageJson = getPackageJson({ workspaceRoot: project.paths.root });
  logger.rootHeader();
  if (project.packageManager !== "yarn") {
    logger.rootStep(
      `adding "workspaces" field to ${project.name} root "package.json"`
    );
    packageJson.workspaces = project.workspaceData.globs;
  }
  logger.rootStep(
    `adding "packageManager" field to ${project.name} root "package.json"`
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
Remove npm workspace data

Removing npm workspaces involves:
  1. Removing the workspaces field from package.json
*/
async function remove(args: RemoveArgs): Promise<void> {
  const { project, options, to, logger } = args;

  logger.mainStep(`Removing npm workspaces`);
  if (to.name !== "yarn") {
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

async function clean(args: CleanArgs): Promise<void> {
  const { project, logger, options } = args;

  logger.subStep(
    `removing ${path.relative(project.paths.root, project.paths.lockfile)}`
  );
  if (!options?.dry) {
    fs.removeSync(project.paths.lockfile);
  }
}

// converts existing, non npm lockfile to a npm lockfile
async function convertLock(args: ConvertArgs): Promise<void> {
  const { project } = args;

  // remove the lockfile
  fs.removeSync(project.paths.lockfile);
}

const npm: ManagerHandler = {
  detect,
  read,
  create,
  remove,
  clean,
  convertLock,
};

export default npm;
