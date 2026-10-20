import path from "node:path";
import { input } from "@inquirer/prompts";
import picocolors from "picocolors";
import { Logger } from "../../logger";
import { directoryInfo } from "../../utils";
import { getWorkspaceDetails } from "../../get-workspace-details";
import type { Workspace } from "../../types";
import type { SummaryCommandArgument } from "./types";

export async function summaryCommand(directory: SummaryCommandArgument) {
  const logger = new Logger();
  logger.hero();

  let selectedDirectory = directory;
  if (!selectedDirectory) {
    selectedDirectory = await input({
      message: "Where is the root of the repo?",
      default: ".",
      validate: (d: string) => {
        const { exists, absolute } = directoryInfo({ directory: d });
        if (exists) {
          return true;
        }
        return `Directory ${picocolors.dim(`(${absolute})`)} does not exist`;
      },
      transformer: (d: string) => d.trim()
    });
    selectedDirectory = selectedDirectory.trim();
  }

  const { exists, absolute: root } = directoryInfo({
    directory: selectedDirectory
  });
  if (!exists) {
    logger.error(`Directory ${picocolors.dim(`(${root})`)} does not exist`);
    return process.exit(1);
  }

  const project = await getWorkspaceDetails({ root });

  const numWorkspaces = project.workspaceData.workspaces.length;
  const hasWorkspaces = numWorkspaces > 0;
  // group workspaces
  const workspacesByDirectory: Record<string, Array<Workspace>> = {};
  for (const workspace of project.workspaceData.workspaces) {
    const workspacePath = path.relative(root, workspace.paths.root);
    const rootDirectory = workspacePath.split(path.sep)[0];
    if (!(rootDirectory in workspacesByDirectory)) {
      workspacesByDirectory[rootDirectory] = [];
    }
    workspacesByDirectory[rootDirectory].push(workspace);
  }

  const renderWorkspace = (w: Workspace) => {
    return `${w.name} (${picocolors.italic(
      `./${path.relative(root, w.paths.root)}`
    )})`;
  };

  const renderDirectory = ({
    number,
    dir,
    workspaces
  }: {
    number: number;
    dir: string;
    workspaces: Array<Workspace>;
  }) => {
    logger.indented(2, `${number}. ${picocolors.bold(dir)}`);
    for (const [idx, workspace] of workspaces.entries()) {
      logger.indented(3, `${idx + 1}. ${renderWorkspace(workspace)}`);
    }
  };

  // repo header
  logger.header("Repository Summary");
  logger.indented(1, `${picocolors.underline(project.name)}:`);
  // workspace manager header
  logger.indented(
    1,
    `Package Manager: ${picocolors.bold(
      picocolors.italic(project.packageManager)
    )}`
  );
  if (hasWorkspaces) {
    // workspaces header
    logger.indented(
      1,
      `Workspaces (${picocolors.bold(numWorkspaces.toString())}):`
    );
    for (const [idx, dir] of Object.keys(workspacesByDirectory).entries()) {
      renderDirectory({
        number: idx + 1,
        workspaces: workspacesByDirectory[dir],
        dir
      });
    }
    logger.blankLine();
  }
}
