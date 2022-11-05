import fs from "fs-extra";
import path from "path";
import execa from "execa";
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
  expandWorkspaces,
  getPnpmWorkspaces,
  getPackageJson,
  updateDependencies,
  getWorkspacePackageManager,
} from "../utils";

// check if a given project using pnpm workspaces
function verify(args: VerifyArgs): boolean {
  const lockFile = path.join(args.workspaceRoot, "pnpm-lock.yaml");
  const workspaceFile = path.join(args.workspaceRoot, "pnpm-workspace.yaml");
  const packageManager = getWorkspacePackageManager({
    workspaceRoot: args.workspaceRoot,
  });
  return (
    fs.existsSync(lockFile) ||
    fs.existsSync(workspaceFile) ||
    packageManager === "pnpm"
  );
}

// read workspace data from pnpm workspaces into generic format
function read(args: ReadArgs): Project {
  return {
    name: getWorkspaceName(args),
    packageManager: "pnpm",
    paths: {
      root: args.workspaceRoot,
      packageJson: path.join(args.workspaceRoot, "package.json"),
      lockfile: path.join(args.workspaceRoot, "pnpm-lock.yaml"),
      workspaceConfig: path.join(args.workspaceRoot, "pnpm-workspace.yaml"),
      nodeModules: path.join(args.workspaceRoot, "node_modules"),
    },
    workspaceData: {
      globs: getPnpmWorkspaces(args),
      workspaces: expandWorkspaces({
        workspaceGlobs: getPnpmWorkspaces(args),
        ...args,
      }),
    },
  };
}

/*
 Create pnpm workspaces from generic format
 Creating pnpm workspaces involves:

  1. Adding the workspaces field in package.json
  2. Setting the packageManager field in package.json
*/
function create(args: CreateArgs) {
  const { project, to, logger, options } = args;

  logger.mainStep(`Creating pnpm workspaces`);
  const packageJson = getPackageJson({ workspaceRoot: project.paths.root });
  packageJson.packageManager = `${to.name}@${to.version}`;
  logger.subStep(
    `adding "packageManager" field to ${project.name} root "package.json"`
  );
  logger.subStep(`adding "pnpm-workspace.yaml"`);

  // write the changes
  if (!options?.dry) {
    fs.writeJSONSync(project.paths.packageJson, packageJson, { spaces: 2 });
    fs.writeFileSync(
      path.join(project.paths.root, "pnpm-workspace.yaml"),
      `packages:\n${project.workspaceData.globs
        .map((w) => `  - "${w}"`)
        .join("\n")}`
    );
  }

  project.workspaceData.workspaces.forEach((workspace) =>
    updateDependencies({ workspace, project, to, logger, options })
  );
}

/*
Remove pnpm workspace data

Cleaning up from pnpm involves:
  1. Removing the pnpm-workspace.yaml file
  2. Removing the package-lock.json file
*/
function remove(args: RemoveArgs): void {
  const { project, logger, options } = args;

  logger.mainStep(`Removing pnpm workspaces`);
  if (project.paths.workspaceConfig) {
    logger.subStep(`removing "pnpm-workspace.yaml"`);
    if (!options?.dry) {
      fs.removeSync(project.paths.workspaceConfig);
    }
  }

  logger.subStep(`removing "node_modules"`);
  if (!options?.dry) {
    fs.rm(project.paths.nodeModules, { recursive: true });
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
  const { project, logger, options } = args;

  logger.subStep(
    `converting ${path.relative(
      project.paths.root,
      project.paths.lockfile
    )} to pnpm-lock.yaml`
  );
  if (!options?.dry) {
    await execa("pnpm", ["import"], {
      stdio: "ignore",
      cwd: args.project.paths.root,
    });
  }
}

const pnpm = {
  verify,
  read,
  create,
  remove,
  clean,
  convertLock,
};

export default pnpm;
