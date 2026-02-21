import path from "node:path";
import fs from "fs-extra";
import {
  input,
  select,
  checkbox,
  confirm as inquirerConfirm,
  Separator
} from "@inquirer/prompts";
import { minimatch } from "minimatch";
import validName from "validate-npm-package-name";
import type { Project, Workspace } from "@turbo/workspaces";
import {
  validateDirectory,
  logger,
  type DependencyGroups,
  type PackageJson
} from "@turbo/utils";
import { getWorkspaceStructure } from "../../utils/getWorkspaceStructure";
import type { WorkspaceType } from "../../generators/types";
import { getWorkspaceList } from "../../utils/getWorkspaceList";

export async function name({
  override,
  suggestion,
  workspaceType
}: {
  override?: string;
  suggestion?: string;
  workspaceType: WorkspaceType;
}): Promise<{ answer: string }> {
  const { validForNewPackages } = validName(override || "");
  if (override && validForNewPackages) {
    return { answer: override };
  }
  const answer = await input({
    message: `What is the name of the ${workspaceType}?`,
    default: suggestion,
    validate: (val: string) => {
      const { validForNewPackages: isValid } = validName(val);
      return isValid || `Invalid ${workspaceType} name`;
    }
  });
  return { answer };
}

export async function type({
  override,
  message
}: {
  override?: WorkspaceType;
  message?: string;
}): Promise<{ answer: WorkspaceType }> {
  if (override) {
    return { answer: override };
  }

  const answer = await select<WorkspaceType>({
    message: message ?? "What type of workspace should be added?",
    choices: [
      {
        name: "app",
        value: "app" as const
      },
      {
        name: "package",
        value: "package" as const
      }
    ]
  });
  return { answer };
}

export async function location({
  workspaceType,
  workspaceName,
  destination,
  project
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
        relative: path.relative(project.paths.root, root)
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

  const answer = await input({
    message: `Where should "${workspaceName}" be added?`,
    default: newWorkspaceLocation
      ? path.relative(project.paths.root, newWorkspaceLocation)
      : undefined,
    validate: (val: string) => {
      const base = path.join(project.paths.root, val);
      const { valid, error } = validateDirectory(base);
      const isWorkspace = project.workspaceData.globs.some((glob) =>
        minimatch(val, glob)
      );

      if (valid && isWorkspace) {
        return true;
      }

      if (!isWorkspace) {
        return `${val} is not a valid workspace location`;
      }

      return error ?? "Invalid directory";
    }
  });

  return {
    absolute: path.join(project.paths.root, answer),
    relative: answer
  };
}

export async function source({
  override,
  workspaces,
  workspaceName
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

  const answer = await select<Workspace>({
    message: `Which workspace should "${workspaceName}" start from?`,
    loop: false,
    pageSize: 25,
    choices: workspaces.map((choice) => {
      if (choice instanceof Separator) {
        return choice;
      }
      return {
        name: `  ${choice.name}`,
        value: choice
      };
    })
  });

  return { answer };
}

export async function dependencies({
  workspaceName,
  project,
  workspaceSource,
  showAllDependencies,
  ...opts
}: {
  workspaceName: string;
  project: Project;
  workspaceSource?: Workspace;
  showAllDependencies?: boolean;
  addDependencies?: boolean;
}) {
  const selectedDependencies: DependencyGroups = {
    dependencies: {},
    devDependencies: {},
    peerDependencies: {},
    optionalDependencies: {}
  };
  if (opts.addDependencies === false) {
    return selectedDependencies;
  }
  const { answer: addDependencies } = await confirm({
    message: `Add workspace dependencies to "${workspaceName}"?`
  });
  if (!addDependencies) {
    return selectedDependencies;
  }

  const dependencyGroups = await checkbox<keyof DependencyGroups>({
    message: `Select all dependencies types to modify for "${workspaceName}"`,
    loop: false,
    choices: [
      { name: "dependencies", value: "dependencies" as const },
      { name: "devDependencies", value: "devDependencies" as const },
      { name: "peerDependencies", value: "peerDependencies" as const },
      { name: "optionalDependencies", value: "optionalDependencies" as const }
    ]
  });

  // supported workspace dependencies (apps can never be dependencies)
  const depChoices = getWorkspaceList({
    project,
    type: "package",
    showAllDependencies
  });

  const sourcePackageJson = workspaceSource
    ? (fs.readJsonSync(workspaceSource.paths.packageJson) as PackageJson)
    : undefined;

  for (const group of dependencyGroups) {
    // eslint-disable-next-line no-await-in-loop -- we want to ask this question group by group
    const selected = await checkbox<string>({
      message: `Which packages should be added as ${group} to "${workspaceName}?`,
      pageSize: 15,
      loop: false,
      choices: depChoices.map((choice) => {
        if (choice instanceof Separator) {
          return choice;
        }
        return {
          name: `  ${choice.name}`,
          value: choice.name
        };
      })
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
          [dep]: project.packageManager === "pnpm" ? "workspace:*" : "*"
        }),
        {}
      );
    }
  }

  return selectedDependencies;
}

export async function confirm({ message }: { message: string }) {
  const answer = await inquirerConfirm({
    message
  });
  return { answer };
}
