import chalk from "chalk";
import type { UtilityArgs } from "../types";

export class Logger {
  transform: string;
  dry: boolean;

  constructor(args: UtilityArgs) {
    this.transform = args.transformer;
    this.dry = args.dry;
  }

  _log(...args: Array<unknown>) {
    // eslint-disable-next-line no-console -- logger
    console.log(...args);
  }

  modified(...args: Array<unknown>) {
    this._log(
      chalk.green(` MODIFIED `),
      ...args,
      this.dry ? chalk.dim(`(dry run)`) : ""
    );
  }
  unchanged(...args: Array<unknown>) {
    this._log(
      chalk.gray(` UNCHANGED `),
      ...args,
      this.dry ? chalk.dim(`(dry run)`) : ""
    );
  }
  skipped(...args: Array<unknown>) {
    this._log(
      chalk.yellow(` SKIPPED `),
      ...args,
      this.dry ? chalk.dim(`(dry run)`) : ""
    );
  }
  error(...args: Array<unknown>) {
    this._log(
      chalk.red(` ERROR `),
      ...args,
      this.dry ? chalk.dim(`(dry run)`) : ""
    );
  }
  info(...args: Array<unknown>) {
    this._log(
      chalk.bold(` INFO `),
      ...args,
      this.dry ? chalk.dim(`(dry run)`) : ""
    );
  }
}
