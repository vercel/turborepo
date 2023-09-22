import path from "node:path";
import type { Project } from "@turbo/workspaces";

// This function is not perfect and could be improved to be more accurate.
// Given a list of workspace globs, it aims to return a selectable list of paths that are valid workspace locations.
// This current naive approach does not work with globs that contain nested wildcards, for example: `packages/*/utils` will not work.
export function getWorkspaceRoots({
  project,
}: {
  project: Project;
}): Array<string> {
  const allWorkspaces = project.workspaceData.workspaces;
  const allWorkspacePaths = allWorkspaces.map((workspace) =>
    path.relative(project.paths.root, workspace.paths.root)
  );

  // find valid workspace locations
  const workspaceRoots = new Set<string>();
  project.workspaceData.globs.forEach((glob) => {
    if (allWorkspacePaths.includes(glob)) {
      // do nothing
    } else if (glob.startsWith("!")) {
      // do nothing
    } else {
      const globParts = glob.split("/");
      const globRoot = globParts[0];
      workspaceRoots.add(globRoot);
    }
  });

  return Array.from(workspaceRoots);
}
