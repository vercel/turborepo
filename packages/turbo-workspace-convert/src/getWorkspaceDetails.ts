import managers from "./managers";
import { Project } from "./types";

async function getWorkspaceDetails({
  workspaceRoot,
  fallback = "yarn",
}: {
  workspaceRoot: string;
  fallback?: keyof typeof managers;
}): Promise<Project> {
  for (const { detect, read } of Object.values(managers)) {
    if (await detect({ workspaceRoot })) {
      return read({ workspaceRoot });
    }
  }

  if (fallback) {
    return managers[fallback].read({ workspaceRoot });
  }

  throw new Error(
    "Could not determine workspace manager. Ensure a lockfile is present."
  );
}

export default getWorkspaceDetails;
