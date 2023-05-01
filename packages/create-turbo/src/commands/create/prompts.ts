import path from "path";
import fs from "fs-extra";
import chalk from "chalk";
import type { PackageManager } from "@turbo/workspaces";
import type { CreateCommandArgument } from "./types";
import { getAvailablePackageManagers } from "@turbo/utils";
import { isFolderEmpty } from "../../utils/isFolderEmpty";
import inquirer from "inquirer";

function validateDirectory(directory: string): {
  valid: boolean;
  root: string;
  projectName: string;
  error?: string;
} {
  const root = path.resolve(directory);
  const projectName = path.basename(root);
  const exists = fs.existsSync(root);

  const stat = fs.lstatSync(root, { throwIfNoEntry: false });
  if (stat && !stat.isDirectory()) {
    return {
      valid: false,
      root,
      projectName,
      error: `${chalk.dim(
        projectName
      )} is not a directory - please try a different location`,
    };
  }

  if (exists) {
    const { isEmpty, conflicts } = isFolderEmpty(root);
    if (!isEmpty) {
      return {
        valid: false,
        root,
        projectName,
        error: `${chalk.dim(projectName)} has ${conflicts.length} conflicting ${
          conflicts.length === 1 ? "file" : "files"
        } - please try a different location`,
      };
    }
  }

  return { valid: true, root, projectName };
}

export async function directory({
  directory,
}: {
  directory: CreateCommandArgument;
}) {
  const projectDirectoryAnswer = await inquirer.prompt<{
    projectDirectory: string;
  }>({
    type: "input",
    name: "projectDirectory",
    message: "Where would you like to create your turborepo?",
    when: !directory,
    default: "./my-turborepo",
    validate: (directory: string) => {
      const { valid, error } = validateDirectory(directory);
      if (!valid && error) {
        return error;
      }
      return true;
    },
    filter: (directory: string) => directory.trim(),
  });

  const { projectDirectory: selectedProjectDirectory = directory as string } =
    projectDirectoryAnswer;

  return validateDirectory(selectedProjectDirectory);
}

export async function packageManager({
  packageManager,
  skipTransforms,
}: {
  packageManager: CreateCommandArgument;
  skipTransforms?: boolean;
}) {
  // if skip transforms is passed, we don't need to ask about the package manager (because that requires a transform)
  if (skipTransforms) {
    return undefined;
  }

  const availablePackageManagers = await getAvailablePackageManagers();
  const packageManagerAnswer = await inquirer.prompt<{
    packageManagerInput?: PackageManager;
  }>({
    name: "packageManagerInput",
    type: "list",
    message: "Which package manager do you want to use?",
    when:
      // prompt for package manager if it wasn't provided as an argument, or if it was
      // provided, but isn't available (always allow npm)
      !packageManager ||
      (packageManager as PackageManager) !== "npm" ||
      !Object.keys(availablePackageManagers).includes(packageManager),
    choices: ["npm", "pnpm", "yarn"].map((p) => ({
      name: p,
      value: p,
      disabled:
        // npm should always be available
        p === "npm" ||
        availablePackageManagers?.[p as PackageManager]?.available
          ? false
          : `not installed`,
    })),
  });

  const {
    packageManagerInput:
      selectedPackageManager = packageManager as PackageManager,
  } = packageManagerAnswer;

  return {
    name: selectedPackageManager,
    version: availablePackageManagers[selectedPackageManager].version,
  };
}
