import type { PackageManager } from "@turbo/utils";
import { getAvailablePackageManagers, validateDirectory } from "@turbo/utils";
import { input, select } from "@inquirer/prompts";
import type { CreateCommandArgument } from "./types";

export async function directory({ dir }: { dir: CreateCommandArgument }) {
  if (dir) {
    return validateDirectory(dir);
  }

  const projectDirectory = await input({
    message: "Where would you like to create your Turborepo?",
    default: "./my-turborepo",
    validate: (d: string) => {
      const { valid, error } = validateDirectory(d);
      if (!valid && error) {
        return error;
      }
      return true;
    },
    transformer: (d: string) => d.trim()
  });

  return validateDirectory(projectDirectory.trim());
}

export async function packageManager({
  manager,
  skipTransforms
}: {
  manager: CreateCommandArgument;
  skipTransforms?: boolean;
}) {
  // if skip transforms is passed, we don't need to ask about the package manager (because that requires a transform)
  if (skipTransforms) {
    return undefined;
  }

  const availablePackageManagers = await getAvailablePackageManagers();

  if (manager && availablePackageManagers[manager as PackageManager]) {
    return {
      name: manager as PackageManager,
      version: availablePackageManagers[manager as PackageManager]
    };
  }

  const selectedPackageManager = await select<PackageManager>({
    message: "Which package manager do you want to use?",
    choices: [
      { pm: "npm", label: "npm" },
      { pm: "pnpm", label: "pnpm" },
      { pm: "yarn", label: "yarn" },
      { pm: "bun", label: "bun" }
    ].map(({ pm, label }) => ({
      name: label,
      value: pm as PackageManager,
      disabled: availablePackageManagers[pm as PackageManager]
        ? false
        : `not installed`
    }))
  });

  return {
    name: selectedPackageManager,
    version: availablePackageManagers[selectedPackageManager]
  };
}
