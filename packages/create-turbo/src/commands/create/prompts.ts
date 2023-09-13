import type { PackageManager } from "@turbo/utils";
import { getAvailablePackageManagers, validateDirectory } from "@turbo/utils";
import inquirer from "inquirer";
import type { CreateCommandArgument } from "./types";

export async function directory({ dir }: { dir: CreateCommandArgument }) {
  const projectDirectoryAnswer = await inquirer.prompt<{
    projectDirectory: string;
  }>({
    type: "input",
    name: "projectDirectory",
    message: "Where would you like to create your turborepo?",
    when: !dir,
    default: "./my-turborepo",
    validate: (d: string) => {
      const { valid, error } = validateDirectory(d);
      if (!valid && error) {
        return error;
      }
      return true;
    },
    filter: (d: string) => d.trim(),
  });

  // eslint-disable-next-line @typescript-eslint/no-non-null-assertion -- we know it's defined because of the `when` condition above
  const { projectDirectory: selectedProjectDirectory = dir! } =
    projectDirectoryAnswer;

  return validateDirectory(selectedProjectDirectory);
}

export async function packageManager({
  manager,
  skipTransforms,
}: {
  manager: CreateCommandArgument;
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
      !manager || !availablePackageManagers[manager as PackageManager],
    choices: [
      { pm: "npm", label: "npm workspaces" },
      { pm: "pnpm", label: "pnpm workspaces" },
      { pm: "yarn", label: "yarn workspaces" },
      { pm: "bun", label: "bun workspaces (beta)" },
    ].map(({ pm, label }) => ({
      name: label,
      value: pm,
      disabled: availablePackageManagers[pm as PackageManager]
        ? false
        : `not installed`,
    })),
  });

  const {
    packageManagerInput: selectedPackageManager = manager as PackageManager,
  } = packageManagerAnswer;

  return {
    name: selectedPackageManager,
    version: availablePackageManagers[selectedPackageManager],
  };
}
