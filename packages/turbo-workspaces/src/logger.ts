import chalk from "chalk";
import gradient from "gradient-string";

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
}
