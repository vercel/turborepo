import managers from "./managers";

function getWorkspaceDetails({
  workspaceRoot,
  fallback = "yarn",
}: {
  workspaceRoot: string;
  fallback?: keyof typeof managers;
}) {
  for (const { verify, read } of Object.values(managers)) {
    if (verify({ workspaceRoot })) {
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
