import path from "node:path";
import inquirer from "inquirer";
import chalk from "chalk";
import { Logger } from "../../logger";
import { directoryInfo } from "../../utils";
import { getWorkspaceDetails } from "../../getWorkspaceDetails";
import type { Workspace } from "../../types";
import type { SummaryCommandArgument } from "./types";

export async function summaryCommand(directory: SummaryCommandArgument) {
  const logger = new Logger();
  logger.hero();

  const answer = await inquirer.prompt<{
    directoryInput?: string;
  }>({
    type: "input",
    name: "directoryInput",
    message: "Where is the root of the repo?",
    when: !directory,
    default: ".",
    validate: (d: string) => {
      const { exists, absolute } = directoryInfo({ directory: d });
      if (exists) {
        return true;
      }
      return `Directory ${chalk.dim(`(${absolute})`)} does not exist`;
    },
    filter: (d: string) => d.trim(),
  });

  const { directoryInput: selectedDirectory = directory } = answer;
  const { exists, absolute: root } = directoryInfo({
    directory: selectedDirectory,
  });
  if (!exists) {
    logger.error(`Directory ${chalk.dim(`(${root})`)} does not exist`);
    return process.exit(1);
  }

  const project = await getWorkspaceDetails({ root });

  const numWorkspaces = project.workspaceData.workspaces.length;
  const hasWorkspaces = numWorkspaces > 0;
  // group workspaces
  const workspacesByDirectory: Record<string, Array<Workspace>> = {};
  project.workspaceData.workspaces.forEach((workspace) => {
    const workspacePath = path.relative(root, workspace.paths.root);
    const rootDirectory = workspacePath.split(path.sep)[0];
    if (!(rootDirectory in workspacesByDirectory)) {
      workspacesByDirectory[rootDirectory] = [];
    }
    workspacesByDirectory[rootDirectory].push(workspace);
  });

  const renderWorkspace = (w: Workspace) => {
    return `${w.name} (${chalk.italic(
      `./${path.relative(root, w.paths.root)}`
    )})`;
  };

  const renderDirectory = ({
    number,
    dir,
    workspaces,
  }: {
    number: number;
    dir: string;
    workspaces: Array<Workspace>;
  }) => {
    logger.indented(2, `${number}. ${chalk.bold(dir)}`);
    workspaces.forEach((workspace, idx) => {
      logger.indented(3, `${idx + 1}. ${renderWorkspace(workspace)}`);
    });
  };

  // repo header
  logger.header(`Repository Summary`);
  logger.indented(1, `${chalk.underline(project.name)}:`);
  // workspace manager header
  logger.indented(
    1,
    `Package Manager: ${chalk.bold(chalk.italic(project.packageManager))}`
  );
  if (hasWorkspaces) {
    // workspaces header
    logger.indented(1, `Workspaces (${chalk.bold(numWorkspaces.toString())}):`);
    Object.keys(workspacesByDirectory).forEach((dir, idx) => {
      renderDirectory({
        number: idx + 1,
        workspaces: workspacesByDirectory[dir],
        dir,
      });
    });
    logger.blankLine();
  }
}
