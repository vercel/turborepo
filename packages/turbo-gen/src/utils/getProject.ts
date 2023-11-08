import { getTurboRoot, logger } from "@turbo/utils";
import { type Project, getWorkspaceDetails } from "@turbo/workspaces";

interface GetProjectArguments {
  root?: string;
}

export async function getProject({
  root,
}: GetProjectArguments): Promise<Project> {
  const directory = root || process.cwd();
  const repoRoot = getTurboRoot(directory);

  if (!repoRoot) {
    logger.error("Unable to infer repository root - override with --root");
  } else {
    try {
      return getWorkspaceDetails({ root: repoRoot });
    } catch (err) {
      logger.error(
        `Unable to determine workspace details. Make sure "${directory}" is the root, or add "packageManager" to "package.json" or ensure a lockfile is present.`
      );
    }
  }

  process.exit(1);
}
