import fs from "fs-extra";
import path from "path";
import {
  VerifyArgs,
  ReadArgs,
  CreateArgs,
  RemoveArgs,
  CleanArgs,
  Project,
  ConvertArgs,
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
function verify(args: VerifyArgs): boolean {
  const lockFile = path.join(args.workspaceRoot, "package-lock.json");
  const packageManager = getWorkspacePackageManager({
    workspaceRoot: args.workspaceRoot,
  });
  return fs.existsSync(lockFile) || packageManager === "npm";
}

/*
 Read workspace data from npm workspaces into generic format
*/
function read(args: ReadArgs): Project {
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
function create(args: CreateArgs) {
  const { project, options, to, logger } = args;

  logger.mainStep(`Creating npm workspaces`);
  const packageJson = getPackageJson({ workspaceRoot: project.paths.root });
  logger.subStep(
    `adding "workspaces" field to ${project.name} root "package.json"`
  );
  packageJson.workspaces = project.workspaceData.globs;
  logger.subStep(
    `adding "packageManager" field to ${project.name} root "package.json"`
  );
  packageJson.packageManager = `${to.name}@${to.version}`;

  if (!options?.dry) {
    fs.writeJSONSync(project.paths.packageJson, packageJson, { spaces: 2 });
  }

  // if we're converting from pnpm, we need to update the workspace package.json files
  if (project.packageManager === "pnpm") {
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
function remove(args: RemoveArgs): void {
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

// converts existing, non pnpm lockfile to a pnpm lockfile
async function convertLock(args: ConvertArgs): Promise<void> {
  return Promise.resolve();
}

const npm = {
  verify,
  read,
  create,
  remove,
  clean,
  convertLock,
};

export default npm;
