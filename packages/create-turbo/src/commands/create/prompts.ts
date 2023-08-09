import type { PackageManager } from "@turbo/workspaces";
import type { CreateCommandArgument } from "./types";
import { getAvailablePackageManagers, validateDirectory } from "@turbo/utils";
import inquirer from "inquirer";

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
      !availablePackageManagers?.[packageManager as PackageManager],
    choices: ["npm", "pnpm", "yarn"].map((p) => ({
      name: p,
      value: p,
      disabled: availablePackageManagers?.[p as PackageManager]
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
    version: availablePackageManagers[selectedPackageManager],
  };
}
