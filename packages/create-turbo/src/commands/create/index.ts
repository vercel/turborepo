import path from "node:path";
import { bold, red, cyan, green } from "picocolors";
import type { Project } from "@turbo/workspaces";
import {
  getWorkspaceDetails,
  install,
  getPackageManagerMeta,
  ConvertError,
} from "@turbo/workspaces";
import {
  getAvailablePackageManagers,
  createProject,
  DownloadError,
  logger,
} from "@turbo/utils";
import { tryGitCommit, tryGitInit, tryGitAdd } from "../../utils/git";
import { isOnline } from "../../utils/isOnline";
import { transforms } from "../../transforms";
import { TransformError } from "../../transforms/errors";
import { isDefaultExample } from "../../utils/isDefaultExample";
import * as prompts from "./prompts";
import type { CreateCommandArgument, CreateCommandOptions } from "./types";

const { turboGradient, turboLoader, info, error, warn } = logger;

function trackOptions(opts: CreateCommandOptions) {
  opts.telemetry?.trackOptionPackageManager(opts.packageManager);
  opts.telemetry?.trackOptionSkipInstall(opts.skipInstall);
  opts.telemetry?.trackOptionSkipTransforms(opts.skipTransforms);
  opts.telemetry?.trackOptionExample(opts.example);
  opts.telemetry?.trackOptionTurboVersion(opts.turboVersion);
  opts.telemetry?.trackOptionExamplePath(opts.examplePath);
}

function handleErrors(
  err: unknown,
  telemetry: CreateCommandOptions["telemetry"]
) {
  telemetry?.trackCommandStatus({ command: "create", status: "error" });
  // handle errors from ../../transforms
  if (err instanceof TransformError) {
    error(bold(err.transform), red(err.message));
    if (err.fatal) {
      process.exit(1);
    }
    // handle errors from @turbo/workspaces
  } else if (err instanceof ConvertError && err.type !== "unknown") {
    error(red(err.message));
    process.exit(1);
    // handle download errors from @turbo/utils
  } else if (err instanceof DownloadError) {
    error(red("Unable to download template from Github"));
    error(red(err.message));
    process.exit(1);
  }

  // handle unknown errors (no special handling, just re-throw to catch at root)
  else {
    throw err;
  }
}

const SCRIPTS_TO_DISPLAY: Record<string, string> = {
  build: "Build",
  dev: "Develop",
  test: "Test",
  lint: "Lint",
};

export async function create(
  directory: CreateCommandArgument,
  opts: CreateCommandOptions
) {
  // track CLI command start
  opts.telemetry?.trackCommandStatus({ command: "create", status: "start" });
  opts.telemetry?.trackArgumentDirectory(Boolean(directory));
  trackOptions(opts);

  const { packageManager, skipInstall, skipTransforms } = opts;

  const [online, availablePackageManagers] = await Promise.all([
    isOnline(),
    getAvailablePackageManagers(),
  ]);

  if (!online) {
    error(
      "You appear to be offline. Please check your network connection and try again."
    );
    process.exit(1);
  }
  const { root, projectName } = await prompts.directory({ dir: directory });
  const relativeProjectDir = path.relative(process.cwd(), root);
  const projectDirIsCurrentDir = relativeProjectDir === "";

  // selected package manager can be undefined if the user chooses to skip transforms
  const selectedPackageManagerDetails = await prompts.packageManager({
    manager: packageManager,
    skipTransforms,
  });

  if (packageManager && opts.skipTransforms) {
    warn(
      "--skip-transforms conflicts with <package-manager>. The package manager argument will be ignored."
    );
  }

  const { example, examplePath } = opts;
  const exampleName = example && example !== "default" ? example : "basic";

  let projectData = {} as Awaited<ReturnType<typeof createProject>>;
  try {
    projectData = await createProject({
      appPath: root,
      example: exampleName,
      isDefaultExample: isDefaultExample(exampleName),
      examplePath,
    });
  } catch (err) {
    handleErrors(err, opts.telemetry);
  }

  const { hasPackageJson, availableScripts, repoInfo } = projectData;

  // create a new git repo after creating the project
  tryGitInit(root, `feat(create-turbo): create ${exampleName}`);

  // read the project after creating it to get details about workspaces, package manager, etc.
  let project: Project = {} as Project;
  try {
    project = await getWorkspaceDetails({ root });
  } catch (err) {
    handleErrors(err, opts.telemetry);
  }

  // run any required transforms
  if (!skipTransforms) {
    for (const transform of transforms) {
      try {
        // eslint-disable-next-line no-await-in-loop -- we need to run transforms sequentially
        const transformResult = await transform({
          example: {
            repo: repoInfo,
            name: exampleName,
          },
          project,
          prompts: {
            projectName,
            root,
            packageManager: selectedPackageManagerDetails,
          },
          opts,
        });

        if (transformResult.result === "success") {
          // add first to ensure any transforms that add new files are included
          tryGitAdd();
          tryGitCommit(
            `feat(create-turbo): apply ${transformResult.name} transform`
          );
        }
      } catch (err) {
        handleErrors(err, opts.telemetry);
      }
    }
  }

  // if the user opted out of transforms, the package manager will be the same as the source example
  const projectPackageManager =
    skipTransforms || !selectedPackageManagerDetails
      ? {
          name: project.packageManager,
          version: availablePackageManagers[project.packageManager],
        }
      : selectedPackageManagerDetails;

  info("Creating a new Turborepo with:");
  logger.log();
  if (project.workspaceData.workspaces.length > 0) {
    const workspacesForDisplay = project.workspaceData.workspaces
      .map((w) => {
        const assignGroupTitle = (relPath: string): string => {
          if (relPath === "apps") {
            return "Application packages";
          }

          if (relPath === "packages") {
            return "Library packages";
          }

          return relPath;
        };
        return {
          group: assignGroupTitle(
            path.relative(root, w.paths.root).split(path.sep)[0] || ""
          ),
          title: path.relative(root, w.paths.root),
          description: w.description,
        };
      })
      .sort((a, b) => a.title.localeCompare(b.title));

    let lastGroup: string | undefined;
    workspacesForDisplay.forEach(({ group, title, description }, idx) => {
      if (idx === 0 || group !== lastGroup) {
        logger.log(cyan(group));
      }
      logger.log(` - ${bold(title)}${description ? `: ${description}` : ""}`);
      lastGroup = group;
    });
  } else {
    logger.log(cyan("apps"));
    logger.log(` - ${bold(projectName)}`);
  }

  // run install
  logger.log();
  if (hasPackageJson && !skipInstall) {
    // in the case when the user opted out of transforms, but not install, we need to make sure the package manager is available
    // before we attempt an install
    if (
      opts.skipTransforms &&
      !availablePackageManagers[project.packageManager]
    ) {
      warn(
        `Unable to install dependencies - "${exampleName}" uses "${project.packageManager}" which could not be found.`
      );
      warn(
        `Try running without "--skip-transforms" to convert "${exampleName}" to a package manager that is available on your system.`
      );
      logger.log();
    } else if (projectPackageManager.version) {
      const loader = turboLoader("Installing dependencies...").start();
      await install({
        project,
        to: projectPackageManager,
        options: {
          interactive: false,
        },
      });

      tryGitCommit("feat(create-turbo): install dependencies");
      loader.stop();
    }
  }

  if (projectDirIsCurrentDir) {
    logger.log(
      `${bold(turboGradient(">>> Success!"))} Your new Turborepo is ready.`
    );
  } else {
    logger.log(
      `${bold(turboGradient(">>> Success!"))} Created your Turborepo at ${green(
        relativeProjectDir
      )}`
    );
  }

  // get the package manager details so we display the right commands to the user in log messages
  const packageManagerMeta = getPackageManagerMeta(projectPackageManager);
  if (packageManagerMeta && hasPackageJson) {
    logger.log();
    logger.log(bold("To get started:"));
    if (!projectDirIsCurrentDir) {
      logger.log(
        `- Change to the directory: ${cyan(`cd ${relativeProjectDir}`)}`
      );
    }
    logger.log(
      `- Enable Remote Caching (recommended): ${cyan(
        `${packageManagerMeta.executable} turbo login`
      )}`
    );
    logger.log(`   - Learn more: https://turbo.build/repo/remote-cache`);
    logger.log();
    logger.log("- Run commands with Turborepo:");
    availableScripts
      .filter((script) => SCRIPTS_TO_DISPLAY[script])
      .forEach((script) => {
        logger.log(
          `   - ${cyan(`${packageManagerMeta.command} run ${script}`)}: ${
            SCRIPTS_TO_DISPLAY[script]
          } all apps and packages`
        );
      });
    logger.log("- Run a command twice to hit cache");
  }
  opts.telemetry?.trackCommandStatus({ command: "create", status: "end" });
}
