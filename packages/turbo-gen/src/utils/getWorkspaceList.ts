import type { Project, Workspace } from "@turbo/workspaces";
import { Separator } from "inquirer";
import type { WorkspaceType } from "../generators/types";
import {
  getWorkspaceStructure,
  getGroupFromWorkspace,
} from "./getWorkspaceStructure";

export function getWorkspaceList({
  project,
  type,
  showAllDependencies,
}: {
  project: Project;
  type: WorkspaceType;
  showAllDependencies?: boolean;
}): Array<Workspace | Separator> {
  const structure = getWorkspaceStructure({ project });
  const workspaceChoices: Array<Workspace | Separator> = [];

  let workspacesForDisplay: Array<Workspace> = project.workspaceData.workspaces;
  if (!showAllDependencies) {
    if (type === "app" && structure.hasRootApps) {
      workspacesForDisplay = structure.workspacesByGroup.apps;
    } else if (type === "package" && structure.nonAppWorkspaces.length > 0) {
      workspacesForDisplay = structure.nonAppWorkspaces;
    }
  }

  // build final list with separators between groups
  let lastGroup: string | undefined;
  workspacesForDisplay.forEach((workspace) => {
    const group = getGroupFromWorkspace({ project, workspace });
    if (group !== lastGroup) {
      workspaceChoices.push(new Separator(group));
    }
    lastGroup = group;
    workspaceChoices.push(workspace);
  });

  return workspaceChoices;
}
