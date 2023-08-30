import path from "node:path";
import type { Project, Workspace } from "@turbo/workspaces";
import { getWorkspaceRoots } from "./workspaceRoots";

interface WorkspaceStructure {
  hasRootApps: boolean;
  hasRootPackages: boolean;
  workspacesByGroup: Record<string, Array<Workspace>>;
  nonAppWorkspaces: Array<Workspace>;
}

export function getGroupFromWorkspace({
  project,
  workspace,
}: {
  project: Project;
  workspace: Workspace;
}) {
  return path
    .relative(project.paths.root, workspace.paths.root)
    .split(path.sep)[0];
}

export function getWorkspaceStructure({
  project,
}: {
  project: Project;
}): WorkspaceStructure {
  // get the workspace roots first, any assumptions we make
  // should at least be based around configured workspaces
  const roots = getWorkspaceRoots({ project });
  const hasRootApps = roots.includes("apps");
  const hasRootPackages = roots.includes("packages");

  const workspacesByGroup: WorkspaceStructure["workspacesByGroup"] = {};
  const nonAppWorkspaces: WorkspaceStructure["nonAppWorkspaces"] = [];
  project.workspaceData.workspaces.forEach((w) => {
    const group = getGroupFromWorkspace({ project, workspace: w });
    if (group !== "apps") {
      nonAppWorkspaces.push(w);
    }

    // add to group
    if (!(group in workspacesByGroup)) {
      workspacesByGroup[group] = [];
    }
    workspacesByGroup[group].push(w);
  });

  return {
    hasRootApps,
    hasRootPackages,
    workspacesByGroup,
    nonAppWorkspaces,
  };
}
