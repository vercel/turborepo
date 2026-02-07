import { input, select } from "@inquirer/prompts";
import picocolors from "picocolors";
import { getAvailablePackageManagers, type PackageManager } from "@turbo/utils";
import { Logger } from "../../logger";
import { directoryInfo } from "../../utils";
import { getWorkspaceDetails } from "../../getWorkspaceDetails";
import { convertProject } from "../../convert";
import type { ConvertCommandArgument, ConvertCommandOptions } from "./types";

function isPackageManagerDisabled({
  packageManager,
  currentWorkspaceManger,
  availablePackageManagers
}: {
  packageManager: PackageManager;
  currentWorkspaceManger: PackageManager;
  availablePackageManagers: Record<PackageManager, string | undefined>;
}) {
  if (currentWorkspaceManger === packageManager) {
    return "already in use";
  }

  if (!availablePackageManagers[packageManager]) {
    return "not installed";
  }

  return false;
}

export async function convertCommand(
  directory: ConvertCommandArgument,
  packageManager: ConvertCommandArgument,
  options: ConvertCommandOptions
) {
  const logger = new Logger(options);

  logger.hero();
  logger.header("Welcome, let's convert your project.");
  logger.blankLine();

  let selectedDirectory = directory;
  if (!selectedDirectory) {
    selectedDirectory = await input({
      message: "Where is the root of your repo?",
      default: ".",
      validate: (d: string) => {
        const { exists, absolute } = directoryInfo({ directory: d });
        if (exists) {
          return true;
        }
        return `Directory ${picocolors.dim(`(${absolute})`)} does not exist`;
      },
      transformer: (d: string) => d.trim()
    });
    selectedDirectory = selectedDirectory.trim();
  }

  const { exists, absolute: root } = directoryInfo({
    directory: selectedDirectory
  });
  if (!exists) {
    logger.error(`Directory ${picocolors.dim(`(${root})`)} does not exist`);
    return process.exit(1);
  }

  const [project, availablePackageManagers] = await Promise.all([
    getWorkspaceDetails({ root }),
    getAvailablePackageManagers()
  ]);

  let selectedPackageManager: PackageManager;
  if (
    packageManager &&
    Object.keys(availablePackageManagers).includes(packageManager)
  ) {
    selectedPackageManager = packageManager as PackageManager;
  } else {
    selectedPackageManager = await select<PackageManager>({
      message: `Convert from ${project.packageManager} to:`,
      choices: [
        { pm: "npm", label: "npm" },
        { pm: "pnpm", label: "pnpm" },
        { pm: "yarn", label: "yarn" },
        { pm: "bun", label: "Bun (beta)" }
      ].map(({ pm, label }) => ({
        name: label,
        value: pm as PackageManager,
        disabled: isPackageManagerDisabled({
          packageManager: pm as PackageManager,
          currentWorkspaceManger: project.packageManager,
          availablePackageManagers
        })
      }))
    });
  }

  await convertProject({
    project,
    convertTo: {
      name: selectedPackageManager,
      // eslint-disable-next-line @typescript-eslint/no-non-null-assertion -- selectedPackageManager is validated against availablePackageManagers
      version: availablePackageManagers[selectedPackageManager]!
    },
    logger,
    options
  });
}
