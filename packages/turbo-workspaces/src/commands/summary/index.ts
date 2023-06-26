import inquirer from "inquirer";
import path from "path";
import { Logger } from "../../logger";
import chalk from "chalk";
import { SummaryCommandArgument } from "./types";
import { directoryInfo } from "../../utils";
import getWorkspaceDetails from "../../getWorkspaceDetails";
import { Workspace } from "../../types";

export default async function summary(directory: SummaryCommandArgument) {
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
    validate: (directory: string) => {
      const { exists, absolute } = directoryInfo({ directory });
      if (exists) {
        return true;
      } else {
        return `Directory ${chalk.dim(`(${absolute})`)} does not exist`;
      }
    },
    filter: (directory: string) => directory.trim(),
  });

  const { directoryInput: selectedDirectory = directory as string } = answer;
  const { exists, absolute: root } = directoryInfo({
    directory: selectedDirectory,
  });
  if (!exists) {
    console.error(`Directory ${chalk.dim(`(${root})`)} does not exist`);
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
    if (!workspacesByDirectory[rootDirectory]) {
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
    directory,
    workspaces,
  }: {
    number: number;
    directory: string;
    workspaces: Array<Workspace>;
  }) => {
    logger.indented(2, `${number}. ${chalk.bold(directory)}`);
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
    Object.keys(workspacesByDirectory).forEach((directory, idx) => {
      renderDirectory({
        number: idx + 1,
        directory,
        workspaces: workspacesByDirectory[directory],
      });
    });
    logger.blankLine();
  }
}
