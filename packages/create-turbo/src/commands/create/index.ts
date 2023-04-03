import path from "path";
import chalk from "chalk";
import type { Project } from "@turbo/workspaces";
import {
  getWorkspaceDetails,
  install,
  getPackageManagerMeta,
  ConvertError,
} from "@turbo/workspaces";
import { getAvailablePackageManagers } from "turbo-utils";
import type { CreateCommandArgument, CreateCommandOptions } from "./types";
import * as prompts from "./prompts";
import { createProject } from "./createProject";
import { tryGitCommit, tryGitInit } from "../../utils/git";
import { isOnline } from "../../utils/isOnline";
import { transforms } from "../../transforms";
import { turboGradient, turboLoader, info, error, warn } from "../../logger";

function handleWorkspaceErrors(err: unknown) {
  if (err instanceof ConvertError && err.type !== "unknown") {
    error(chalk.red(err.message));
    process.exit(1);
  } else {
    // if it's an unknown error type, re-throw to root
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
  packageManager: CreateCommandArgument,
  opts: CreateCommandOptions
) {
  const { skipInstall, skipTransforms } = opts;
  console.log(chalk.bold(turboGradient(`\n>>> TURBOREPO\n`)));
  info(`Welcome to Turborepo! Let's get you set up with a new codebase.`);
  console.log();

  const availablePackageManagers = await getAvailablePackageManagers();
  const online = await isOnline();
  if (!online) {
    error(
      "You appear to be offline. Please check your network connection and try again."
    );
    process.exit(1);
  }
  const { root, projectName } = await prompts.directory({ directory });
  const relativeProjectDir = path.relative(process.cwd(), root);
  const projectDirIsCurrentDir = relativeProjectDir === "";

  // selected package manager can be undefined if the user chooses to skip transforms
  const selectedPackageManagerDetails = await prompts.packageManager({
    packageManager,
    skipTransforms,
  });

  if (packageManager && opts.skipTransforms) {
    warn(
      '"--skip-transforms" flag conflicts with the package manager argument. The package manager argument will be ignored.'
    );
  }

  const { example, examplePath } = opts;
  const exampleName = example && example !== "default" ? example : "basic";
  const { hasPackageJson, availableScripts, repoInfo } = await createProject({
    appPath: root,
    example: exampleName,
    examplePath,
  });

  // create a new git repo after creating the project
  tryGitInit(root, `feat(create-turbo): create ${exampleName}`);

  let project: Project = {} as Project;
  try {
    project = await getWorkspaceDetails({ root });
  } catch (err) {
    handleWorkspaceErrors(err);
  }

  let successfulTransforms = 0;
  if (!skipTransforms) {
    for (const transform of transforms) {
      try {
        const result = await transform({
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
        if (result.result === "success") {
          successfulTransforms += 1;
        }
      } catch (err) {
        handleWorkspaceErrors(err);
      }
    }
  }

  if (successfulTransforms > 0) {
    // create a second commit after running transforms
    tryGitCommit("feat(create-turbo): apply transforms");
  }

  // if the user opted out of transforms, the package manager will be the same as the example
  const projectPackageManager =
    skipTransforms || !selectedPackageManagerDetails
      ? {
          name: project.packageManager,
          version: availablePackageManagers[project.packageManager].version,
        }
      : selectedPackageManagerDetails;

  info("Created a new Turborepo with the following:");
  console.log();
  if (project.workspaceData.workspaces.length > 0) {
    const workspacesForDisplay = project.workspaceData.workspaces
      .map((w) => ({
        group: path.relative(root, w.paths.root).split(path.sep)?.[0] || "",
        title: path.relative(root, w.paths.root),
        description: w.description,
      }))
      .sort((a, b) => a.title.localeCompare(b.title));

    let lastGroup: string | undefined;
    workspacesForDisplay.forEach(({ group, title, description }, idx) => {
      if (idx === 0 || group !== lastGroup) {
        console.log(chalk.cyan(group));
      }
      console.log(
        ` - ${chalk.bold(title)}${description ? `: ${description}` : ""}`
      );
      lastGroup = group;
    });
  } else {
    console.log(chalk.cyan("apps"));
    console.log(` - ${chalk.bold(projectName)}`);
  }

  console.log();
  if (hasPackageJson && !skipInstall) {
    if (
      opts.skipTransforms &&
      !availablePackageManagers[project.packageManager].available
    ) {
      warn(
        `Unable to install dependencies - "${exampleName}" uses "${project.packageManager}" which could not be found.`
      );
      warn(
        `Try running without "--skip-transforms" to convert "${exampleName}" to a package manager that is available on your system.`
      );
      console.log();
    } else if (selectedPackageManagerDetails) {
      console.log("Installing packages. This might take a couple of minutes.");
      console.log();

      const loader = turboLoader("Installing dependencies...").start();
      await install({
        project,
        to: selectedPackageManagerDetails,
        options: {
          interactive: false,
        },
      });

      tryGitCommit("feat(create-turbo): install dependencies");
      loader.stop();
    }
  }

  if (projectDirIsCurrentDir) {
    console.log(
      `${chalk.bold(
        turboGradient(">>> Success!")
      )} Your new Turborepo is ready.`
    );
  } else {
    console.log(
      `${chalk.bold(
        turboGradient(">>> Success!")
      )} Created a new Turborepo at "${relativeProjectDir}".`
    );
  }

  // find the right package manager details to display in log messages
  const packageManagerMeta = getPackageManagerMeta(projectPackageManager);
  if (packageManagerMeta && hasPackageJson) {
    if (projectDirIsCurrentDir) {
      console.log("Inside this directory, you can run several commands:");
    } else {
      console.log("Inside that directory, you can run several commands:");
    }
    console.log();
    availableScripts
      .filter((script) => SCRIPTS_TO_DISPLAY[script])
      .forEach((script) => {
        console.log(
          chalk.cyan(`  ${packageManagerMeta.command} run ${script}`)
        );
        console.log(`     ${SCRIPTS_TO_DISPLAY[script]} all apps and packages`);
        console.log();
      });
    console.log(`Turborepo will cache locally by default. For an additional`);
    console.log(`speed boost, enable Remote Caching with Vercel by`);
    console.log(`entering the following command:`);
    console.log();
    console.log(chalk.cyan(`  ${packageManagerMeta.executable} turbo login`));
    console.log();
    console.log(`We suggest that you begin by typing:`);
    console.log();
    if (!projectDirIsCurrentDir) {
      console.log(`  ${chalk.cyan("cd")} ${relativeProjectDir}`);
    }
    console.log(chalk.cyan(`  ${packageManagerMeta.executable} turbo login`));
    console.log();
  }
}
