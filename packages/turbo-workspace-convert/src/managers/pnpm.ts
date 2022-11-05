import fs from "fs-extra";
import path from "path";
import execa from "execa";
import {
  DetectArgs,
  ReadArgs,
  CreateArgs,
  RemoveArgs,
  ConvertArgs,
  CleanArgs,
  Project,
  ManagerHandler,
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
async function detect(args: DetectArgs): Promise<boolean> {
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
async function read(args: ReadArgs): Promise<Project> {
  const isPnpm = await detect(args);
  if (!isPnpm) {
    throw new Error("Not a pnpm workspaces project");
  }

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
async function create(args: CreateArgs): Promise<void> {
  const { project, to, logger, options } = args;

  logger.mainStep(`Creating pnpm workspaces`);
  const packageJson = getPackageJson({ workspaceRoot: project.paths.root });
  logger.rootHeader();
  packageJson.packageManager = `${to.name}@${to.version}`;
  logger.rootStep(
    `adding "packageManager" field to ${project.name} root "package.json"`
  );
  logger.rootStep(`adding "pnpm-workspace.yaml"`);

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

/*
Remove pnpm workspace data

Cleaning up from pnpm involves:
  1. Removing the pnpm-workspace.yaml file
  2. Removing the package-lock.json file
*/
async function remove(args: RemoveArgs): Promise<void> {
  const { project, logger, options } = args;

  logger.mainStep(`Removing pnpm workspaces`);
  if (project.paths.workspaceConfig) {
    logger.subStep(`removing "pnpm-workspace.yaml"`);
    if (!options?.dry) {
      fs.removeSync(project.paths.workspaceConfig);
    }
  }

  if (!options?.dry) {
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

// converts existing, non-pnpm lockfile to a pnpm lockfile
async function convertLock(args: ConvertArgs): Promise<void> {
  const { project, logger, options } = args;

  if (project.packageManager !== "pnpm") {
    logger.subStep(
      `converting ${path.relative(
        project.paths.root,
        project.paths.lockfile
      )} to pnpm-lock.yaml`
    );
    if (!options?.dry && fs.existsSync(project.paths.lockfile)) {
      await execa("pnpm", ["import"], {
        stdio: "ignore",
        cwd: project.paths.root,
      });
    }
  }
}

const pnpm: ManagerHandler = {
  detect,
  read,
  create,
  remove,
  clean,
  convertLock,
};

export default pnpm;
