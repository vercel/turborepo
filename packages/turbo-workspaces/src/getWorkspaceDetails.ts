import { ConvertError } from "./errors";
import managers from "./managers";
import { Project } from "./types";
import { directoryInfo } from "./utils";

export default async function getWorkspaceDetails({
  root,
}: {
  root: string;
}): Promise<Project> {
  const { exists, absolute: workspaceRoot } = directoryInfo({
    directory: root,
  });
  if (!exists) {
    throw new ConvertError(
      `Could not find directory at ${workspaceRoot}. Ensure the directory exists.`,
      {
        type: "invalid_directory",
      }
    );
  }

  for (const { detect, read } of Object.values(managers)) {
    if (await detect({ workspaceRoot })) {
      return read({ workspaceRoot });
    }
  }

  throw new ConvertError(
    "Could not determine package manager. Add `packageManager` to `package.json` or ensure a lockfile is present.",
    {
      type: "package_manager-unable_to_detect",
    }
  );
}
