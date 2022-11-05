import fs from "fs-extra";
import path from "path";
import {
  VerifyArgs,
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
function verify(args: VerifyArgs): boolean {
  const lockFile = path.join(args.workspaceRoot, "yarn.lock");
  const packageManager = getWorkspacePackageManager({
    workspaceRoot: args.workspaceRoot,
  });
  return fs.existsSync(lockFile) || packageManager === "yarn";
}

/*
 read workspace data from yarn workspaces into generic format
*/
function read(args: ReadArgs): Project {
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
function create(args: CreateArgs) {
  const { project, to, logger, options } = args;

  logger.mainStep(`Creating yarn workspaces`);
  const packageJson = getPackageJson({ workspaceRoot: project.paths.root });
  logger.rootHeader();

  // workspaces
  logger.rootStep(
    `adding "workspaces" field to ${path.relative(
      project.paths.root,
      project.paths.packageJson
    )}`
  );
  packageJson.workspaces = project.workspaceData.globs;

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
function remove(args: RemoveArgs): void {
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
    }

    logger.subStep(`removing "node_modules"`);
    if (!options?.dry) {
      fs.rm(project.paths.nodeModules, { recursive: true });
    }
  }
}

function clean(args: CleanArgs): void {
  const { project, logger, options } = args;

  logger.subStep(
    `removing ${path.relative(project.paths.root, project.paths.lockfile)}`
  );
  if (!options?.dry) {
    fs.removeSync(project.paths.lockfile);
  }
}

/*
converts existing, non pnpm lockfile to a pnpm lockfile
*/
async function convertLock(args: ConvertArgs): Promise<void> {
  return Promise.resolve();
}

const yarn = {
  verify,
  read,
  create,
  remove,
  clean,
  convertLock,
};

export default yarn;
