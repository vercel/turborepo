import fs from "fs-extra";
import path from "path";
import inquirer from "inquirer";
import { minimatch } from "minimatch";
import validName from "validate-npm-package-name";
import type { Project, Workspace } from "@turbo/workspaces";
import {
  validateDirectory,
  logger,
  type DependencyGroups,
  type PackageJson,
} from "@turbo/utils";
import { getWorkspaceStructure } from "../../utils/getWorkspaceStructure";
import type { WorkspaceType } from "../../generators/types";
import { getWorkspaceList } from "../../utils/getWorkspaceList";

export async function name({
  override,
  suggestion,
  type,
}: {
  override?: string;
  suggestion?: string;
  type: WorkspaceType;
}): Promise<{ answer: string }> {
  const { validForNewPackages } = validName(override || "");
  if (override && validForNewPackages) {
    return { answer: override };
  }
  return inquirer.prompt<{ answer: string }>({
    type: "input",
    name: "answer",
    default: suggestion,
    validate: (input: string) => {
      const { validForNewPackages } = validName(input);
      return validForNewPackages || `Invalid ${type} name`;
    },
    message: `What is the name of the ${type}?`,
  });
}

export async function type({
  override,
  message,
}: {
  override?: WorkspaceType;
  message?: string;
}): Promise<{ answer: WorkspaceType }> {
  if (override) {
    return { answer: override };
  }

  return inquirer.prompt<{ answer: WorkspaceType }>({
    type: "list",
    name: "answer",
    message: message ?? `What type of workspace should be added?`,
    choices: [
      {
        name: "app",
        value: "app",
      },
      {
        name: "package",
        value: "package",
      },
    ],
  });
}

export async function location({
  type,
  name,
  destination,
  project,
}: {
  type: "app" | "package";
  name: string;
  destination?: string;
  project: Project;
}): Promise<{ absolute: string; relative: string }> {
  // handle names with scopes
  const nameAsPath = name.includes("/") ? name.split("/")[1] : name;

  // handle destination option (NOTE: this intentionally allows adding packages to non workspace directories)
  if (destination) {
    const { valid, root } = validateDirectory(destination);
    if (valid) {
      return {
        absolute: root,
        relative: path.relative(project.paths.root, root),
      };
    }
  }

  // build default name based on what is being added
  let newWorkspaceLocation: string | undefined = undefined;
  const workspaceStructure = getWorkspaceStructure({ project });

  if (type === "app" && workspaceStructure.hasRootApps) {
    newWorkspaceLocation = `${project.paths.root}/apps/${nameAsPath}`;
  } else if (type === "package" && workspaceStructure.hasRootPackages) {
    newWorkspaceLocation = `${project.paths.root}/packages/${nameAsPath}`;
  }

  const { answer } = await inquirer.prompt<{
    answer: string;
  }>({
    type: "input",
    name: "answer",
    message: `Where should "${name}" be added?`,
    default: newWorkspaceLocation
      ? path.relative(project.paths.root, newWorkspaceLocation)
      : undefined,
    validate: (input: string) => {
      const base = path.join(project.paths.root, input);
      const { valid, error } = validateDirectory(base);
      const isWorkspace = project.workspaceData.globs.some((glob) =>
        minimatch(input, glob)
      );

      if (valid && isWorkspace) {
        return true;
      }

      if (!isWorkspace) {
        return `${input} is not a valid workspace location`;
      }

      return error;
    },
  });

  return {
    absolute: path.join(project.paths.root, answer),
    relative: answer,
  };
}

export async function source({
  override,
  workspaces,
  name,
}: {
  override?: string;
  workspaces: Array<Workspace | inquirer.Separator>;
  name: string;
}) {
  if (override) {
    const source = workspaces.find((workspace) => {
      if (workspace instanceof inquirer.Separator) {
        return false;
      }
      return workspace.name === override;
    }) as Workspace | undefined;
    if (source) {
      return { answer: source };
    }
    logger.warn(`Workspace "${override}" not found`);
    console.log();
  }

  const sourceAnswer = await inquirer.prompt<{
    answer: Workspace;
  }>({
    type: "list",
    name: "answer",
    loop: false,
    pageSize: 25,
    message: `Which workspace should "${name}" start from?`,
    choices: workspaces.map((choice) => {
      if (choice instanceof inquirer.Separator) {
        return choice;
      }
      return {
        name: `  ${choice.name}`,
        value: choice,
      };
    }),
  });

  return sourceAnswer;
}

export async function dependencies({
  name,
  project,
  source,
  showAllDependencies,
}: {
  name: string;
  project: Project;
  source?: Workspace;
  showAllDependencies?: boolean;
}) {
  const selectedDependencies: DependencyGroups = {
    dependencies: {},
    devDependencies: {},
    peerDependencies: {},
    optionalDependencies: {},
  };
  const { answer: addDependencies } = await confirm({
    message: `Add workspace dependencies to "${name}"?`,
  });
  if (!addDependencies) {
    return selectedDependencies;
  }

  const { answer: dependencyGroups } = await inquirer.prompt<{
    answer: Array<keyof DependencyGroups>;
  }>({
    type: "checkbox",
    name: "answer",
    message: `Select all dependencies types to modify for "${name}"`,
    loop: false,
    choices: [
      { name: "dependencies", value: "dependencies" },
      { name: "devDependencies", value: "devDependencies" },
      { name: "peerDependencies", value: "peerDependencies" },
      { name: "optionalDependencies", value: "optionalDependencies" },
    ],
  });

  // supported workspace dependencies (apps can never be dependencies)
  let depChoices = getWorkspaceList({
    project,
    type: "package",
    showAllDependencies,
  });

  const sourcePackageJson = source
    ? (fs.readJsonSync(source.paths.packageJson) as PackageJson)
    : undefined;

  for (let group of dependencyGroups) {
    const { answer: selected } = await inquirer.prompt<{
      answer: Array<string>;
    }>({
      type: "checkbox",
      name: "answer",
      default:
        sourcePackageJson && Object.keys(sourcePackageJson?.[group] || {}),
      pageSize: 15,
      message: `Which packages should be added as ${group} to "${name}?`,
      loop: false,
      choices: depChoices.map((choice) => {
        if (choice instanceof inquirer.Separator) {
          return choice;
        }
        return {
          name: `  ${choice.name}`,
          value: choice.name,
        };
      }),
    });

    const newDependencyGroup = sourcePackageJson?.[group] || {};
    if (Object.keys(newDependencyGroup).length) {
      const existingDependencyKeys = new Set(Object.keys(newDependencyGroup));

      selected.forEach((dep) => {
        if (!existingDependencyKeys.has(dep)) {
          newDependencyGroup[dep] =
            project.packageManager === "pnpm" ? "workspace:*" : "*";
        }
      });

      selectedDependencies[group] = newDependencyGroup;
    } else {
      selectedDependencies[group] = selected.reduce(
        (acc, dep) => ({
          ...acc,
          [dep]: project.packageManager === "pnpm" ? "workspace:*" : "*",
        }),
        {}
      );
    }
  }

  return selectedDependencies;
}

export async function confirm({ message }: { message: string }) {
  return await inquirer.prompt<{ answer: boolean }>({
    type: "confirm",
    name: "answer",
    message,
  });
}
