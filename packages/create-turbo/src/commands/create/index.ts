import path from "path";
import chalk from "chalk";
import type { Project } from "@turbo/workspaces";
import {
  convert,
  getWorkspaceDetails,
  install,
  getPackageManagerMeta,
  ConvertError,
} from "@turbo/workspaces";
import type { CreateCommandArgument, CreateCommandOptions } from "./types";
import * as prompts from "./prompts";
import { createProject } from "./createProject";
import { tryGitInit } from "../../utils/git";
import { isOnline } from "../../utils/isOnline";
import { turboGradient, turboLoader, info, error } from "../../logger";

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
  const { skipInstall } = opts;
  console.log(chalk.bold(turboGradient(`\n>>> TURBOREPO\n`)));
  info(`Welcome to Turborepo! Let's get you set up with a new codebase.`);
  console.log();

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

  const selectedPackageManagerDetails = await prompts.packageManager({
    packageManager,
  });

  const { example, examplePath } = opts;

  const { hasPackageJson, availableScripts } = await createProject({
    appPath: root,
    projectName,
    example: example && example !== "default" ? example : "basic",
    examplePath,
  });

  let project: Project = {} as Project;
  try {
    project = await getWorkspaceDetails({ root });
  } catch (err) {
    handleWorkspaceErrors(err);
  }

  if (project.packageManager !== selectedPackageManagerDetails.name) {
    try {
      await convert({
        root,
        to: selectedPackageManagerDetails.name,
        options: {
          // skip install after conversion- we will do it later
          skipInstall: true,
        },
      });
    } catch (err) {
      handleWorkspaceErrors(err);
    }
  }

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
    loader.stop();
  }

  // once we're done moving things around, init a new repo
  tryGitInit(root);

  if (projectDirIsCurrentDir) {
    console.log(
      `${chalk.bold(
        turboGradient(">>> Success!")
      )} Your new Turborepo is ready.`
    );
    console.log("Inside this directory, you can run several commands:");
  } else {
    console.log(
      `${chalk.bold(
        turboGradient(">>> Success!")
      )} Created a new Turborepo at "${relativeProjectDir}".`
    );
    console.log("Inside that directory, you can run several commands:");
  }

  const packageManagerMeta = getPackageManagerMeta(
    selectedPackageManagerDetails
  );
  if (packageManagerMeta && hasPackageJson) {
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
