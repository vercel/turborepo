import { bold, green, red, underline, yellow } from "picocolors";
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

  logger(...args: Array<unknown>) {
    if (this.interactive) {
      // eslint-disable-next-line no-console -- logger
      console.log(...args);
    }
  }

  indented(level: number, ...args: Array<unknown>) {
    this.logger(" ".repeat(INDENTATION * level), ...args);
  }

  header(title: string) {
    this.blankLine();
    this.logger(bold(title));
  }

  installerFrames() {
    const prefix = `${" ".repeat(INDENTATION)} - ${
      this.dry ? yellow("SKIPPED | ") : green("OK | ")
    }`;
    return [`${prefix}   `, `${prefix}>  `, `${prefix}>> `, `${prefix}>>>`];
  }

  gradient(text: string | number) {
    const turboGradient = gradient("#0099F7", "#F11712");
    return turboGradient(text.toString());
  }

  hero() {
    this.logger(bold(this.gradient(`\n>>> TURBOREPO\n`)));
  }

  info(...args: Array<unknown>) {
    this.logger(...args);
  }

  mainStep(title: string) {
    this.blankLine();
    this.logger(`${this.step}. ${underline(title)}`);
    this.step += 1;
  }

  subStep(...args: Array<unknown>) {
    this.logger(
      " ".repeat(INDENTATION),
      `-`,
      this.dry ? yellow("SKIPPED |") : green("OK |"),
      ...args
    );
  }

  subStepFailure(...args: Array<unknown>) {
    this.logger(" ".repeat(INDENTATION), `-`, red("ERROR |"), ...args);
  }

  rootHeader() {
    this.blankLine();
    this.indented(2, "Root:");
  }

  rootStep(...args: Array<unknown>) {
    this.logger(
      " ".repeat(INDENTATION * 3),
      `-`,
      this.dry ? yellow("SKIPPED |") : green("OK |"),
      ...args
    );
  }

  workspaceHeader() {
    this.blankLine();
    this.indented(2, "Workspaces:");
  }

  workspaceStep(...args: Array<unknown>) {
    this.logger(
      " ".repeat(INDENTATION * 3),
      `-`,
      this.dry ? yellow("SKIPPED |") : green("OK |"),
      ...args
    );
  }

  blankLine() {
    this.logger();
  }

  error(...args: Array<unknown>) {
    // eslint-disable-next-line no-console -- logger
    console.error(...args);
  }
}
