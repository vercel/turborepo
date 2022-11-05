import chalk from "chalk";
import path from "path";
import gradient from "gradient-string";
import { Workspace, Project } from "./types";

const INDENTATION = 2;

export class Logger {
  interactive: boolean;
  dry: boolean;
  step: number;

  constructor({
    interactive,
    dry,
  }: { interactive?: boolean; dry?: boolean } = {}) {
    this.interactive = interactive ?? true;
    this.dry = dry ?? false;
    this.step = 1;
  }

  logger(...args: any[]) {
    if (this.interactive) {
      console.log(...args);
    }
  }

  indented(level: number, ...args: any[]) {
    this.logger(" ".repeat(INDENTATION * level), ...args);
  }

  header(title: string) {
    this.blankLine();
    this.logger(chalk.bold(title));
  }

  installerFrames() {
    const prefix = `${" ".repeat(INDENTATION)} - ${
      this.dry ? chalk.yellow("SKIPPED | ") : chalk.green("OK | ")
    }`;
    return [`${prefix}   `, `${prefix}>  `, `${prefix}>> `, `${prefix}>>>`];
  }

  gradient(text: string | number) {
    const turboGradient = gradient("#0099F7", "#F11712");
    return turboGradient(text.toString());
  }

  hero() {
    this.logger(chalk.bold(this.gradient(`\n>>> TURBOREPO\n`)));
  }

  info(...args: any[]) {
    this.logger(...args);
  }

  mainStep(title: string) {
    this.blankLine();
    this.logger(`${this.step}. ${chalk.underline(title)}`);
    this.step += 1;
  }

  subStep(...args: any[]) {
    this.logger(
      " ".repeat(INDENTATION),
      `-`,
      this.dry ? chalk.yellow("SKIPPED |") : chalk.green("OK |"),
      ...args
    );
  }

  subStepFailure(...args: any[]) {
    this.logger(" ".repeat(INDENTATION), `-`, chalk.red("ERROR |"), ...args);
  }

  rootHeader() {
    this.blankLine();
    this.indented(2, "Root:");
  }

  rootStep(...args: any[]) {
    this.logger(
      " ".repeat(INDENTATION * 3),
      `-`,
      this.dry ? chalk.yellow("SKIPPED |") : chalk.green("OK |"),
      ...args
    );
  }

  workspaceHeader() {
    this.blankLine();
    this.indented(2, "Workspaces:");
  }

  workspaceStep(...args: any[]) {
    this.logger(
      " ".repeat(INDENTATION * 3),
      `-`,
      this.dry ? chalk.yellow("SKIPPED |") : chalk.green("OK |"),
      ...args
    );
  }

  blankLine() {
    this.logger();
  }

  error(...args: any[]) {
    console.error(...args);
  }

  workspaceSummary({
    workspaceRoot,
    project,
  }: {
    workspaceRoot: string;
    project: Project;
  }) {
    const numWorkspaces = project.workspaceData.workspaces.length.toString();

    // group workspaces
    const workspacesByDirectory: Record<string, Array<Workspace>> = {};
    project.workspaceData.workspaces.forEach((workspace) => {
      const workspacePath = path.relative(workspaceRoot, workspace.paths.root);
      const rootDirectory = workspacePath.split(path.sep)[0];
      if (!workspacesByDirectory[rootDirectory]) {
        workspacesByDirectory[rootDirectory] = [];
      }
      workspacesByDirectory[rootDirectory].push(workspace);
    });

    const renderWorkspace = (w: Workspace) => {
      return `${w.name} (${chalk.italic(
        `./${path.relative(workspaceRoot, w.paths.root)}`
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
      this.indented(2, `${number}. ${chalk.bold(directory)}`);
      workspaces.forEach((workspace, idx) => {
        this.indented(3, `${idx + 1}. ${renderWorkspace(workspace)}`);
      });
    };

    // repo header
    this.header(`Repository Summary`);
    this.indented(1, `${chalk.underline(project.name)}:`);
    // workspace manager header
    this.indented(
      1,
      `Workspace Manager: ${chalk.bold(chalk.italic(project.packageManager))}`
    );
    // workspaces header
    this.indented(1, `Workspaces (${chalk.bold(numWorkspaces)}):`);
    Object.keys(workspacesByDirectory).forEach((directory, idx) => {
      renderDirectory({
        number: idx + 1,
        directory,
        workspaces: workspacesByDirectory[directory],
      });
    });
    this.blankLine();
  }
}
