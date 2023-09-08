import path from "node:path";
import { readJsonSync } from "fs-extra";
import { prompt, Separator } from "inquirer";
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
  workspaceType,
}: {
  override?: string;
  suggestion?: string;
  workspaceType: WorkspaceType;
}): Promise<{ answer: string }> {
  const { validForNewPackages } = validName(override || "");
  if (override && validForNewPackages) {
    return { answer: override };
  }
  return prompt<{ answer: string }>({
    type: "input",
    name: "answer",
    default: suggestion,
    validate: (input: string) => {
      const { validForNewPackages: isValid } = validName(input);
      return isValid || `Invalid ${workspaceType} name`;
    },
    message: `What is the name of the ${workspaceType}?`,
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

  return prompt<{ answer: WorkspaceType }>({
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
  workspaceType,
  workspaceName,
  destination,
  project,
}: {
  workspaceType: WorkspaceType;
  workspaceName: string;
  destination?: string;
  project: Project;
}): Promise<{ absolute: string; relative: string }> {
  // handle names with scopes
  const nameAsPath = workspaceName.includes("/")
    ? workspaceName.split("/")[1]
    : workspaceName;

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
  let newWorkspaceLocation: string | undefined;
  const workspaceStructure = getWorkspaceStructure({ project });

  if (workspaceType === "app" && workspaceStructure.hasRootApps) {
    newWorkspaceLocation = `${project.paths.root}/apps/${nameAsPath}`;
  } else if (
    workspaceType === "package" &&
    workspaceStructure.hasRootPackages
  ) {
    newWorkspaceLocation = `${project.paths.root}/packages/${nameAsPath}`;
  }

  const { answer } = await prompt<{
    answer: string;
  }>({
    type: "input",
    name: "answer",
    message: `Where should "${workspaceName}" be added?`,
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
  workspaceName,
}: {
  override?: string;
  workspaces: Array<Workspace | Separator>;
  workspaceName: string;
}) {
  if (override) {
    const workspaceSource = workspaces.find((workspace) => {
      if (workspace instanceof Separator) {
        return false;
      }
      return workspace.name === override;
    }) as Workspace | undefined;
    if (workspaceSource) {
      return { answer: workspaceSource };
    }
    logger.warn(`Workspace "${override}" not found`);
    logger.log();
  }

  const sourceAnswer = await prompt<{
    answer: Workspace;
  }>({
    type: "list",
    name: "answer",
    loop: false,
    pageSize: 25,
    message: `Which workspace should "${workspaceName}" start from?`,
    choices: workspaces.map((choice) => {
      if (choice instanceof Separator) {
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
  workspaceName,
  project,
  workspaceSource,
  showAllDependencies,
}: {
  workspaceName: string;
  project: Project;
  workspaceSource?: Workspace;
  showAllDependencies?: boolean;
}) {
  const selectedDependencies: DependencyGroups = {
    dependencies: {},
    devDependencies: {},
    peerDependencies: {},
    optionalDependencies: {},
  };
  const { answer: addDependencies } = await confirm({
    message: `Add workspace dependencies to "${workspaceName}"?`,
  });
  if (!addDependencies) {
    return selectedDependencies;
  }

  const { answer: dependencyGroups } = await prompt<{
    answer: Array<keyof DependencyGroups>;
  }>({
    type: "checkbox",
    name: "answer",
    message: `Select all dependencies types to modify for "${workspaceName}"`,
    loop: false,
    choices: [
      { name: "dependencies", value: "dependencies" },
      { name: "devDependencies", value: "devDependencies" },
      { name: "peerDependencies", value: "peerDependencies" },
      { name: "optionalDependencies", value: "optionalDependencies" },
    ],
  });

  // supported workspace dependencies (apps can never be dependencies)
  const depChoices = getWorkspaceList({
    project,
    type: "package",
    showAllDependencies,
  });

  const sourcePackageJson = workspaceSource
    ? (readJsonSync(workspaceSource.paths.packageJson) as PackageJson)
    : undefined;

  for (const group of dependencyGroups) {
    // eslint-disable-next-line no-await-in-loop -- we want to ask this question group by group
    const { answer: selected } = await prompt<{
      answer: Array<string>;
    }>({
      type: "checkbox",
      name: "answer",
      default: sourcePackageJson && Object.keys(sourcePackageJson[group] || {}),
      pageSize: 15,
      message: `Which packages should be added as ${group} to "${workspaceName}?`,
      loop: false,
      choices: depChoices.map((choice) => {
        if (choice instanceof Separator) {
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
  return prompt<{ answer: boolean }>({
    type: "confirm",
    name: "answer",
    message,
  });
}
