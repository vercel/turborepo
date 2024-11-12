import { green, dim, bold, red, yellow, gray } from "picocolors";
import type { UtilityArgs } from "../types";

export class Logger {
  transform: string;
  dry: boolean;

  constructor(args: UtilityArgs) {
    this.transform = args.transformer;
    this.dry = args.dryRun;
  }

  _log(...args: Array<unknown>) {
    // eslint-disable-next-line no-console -- logger
    console.log(...args);
  }

  modified(...args: Array<unknown>) {
    this._log(green(` MODIFIED `), ...args, this.dry ? dim(`(dry run)`) : "");
  }
  unchanged(...args: Array<unknown>) {
    this._log(gray(` UNCHANGED `), ...args, this.dry ? dim(`(dry run)`) : "");
  }
  skipped(...args: Array<unknown>) {
    this._log(yellow(` SKIPPED `), ...args, this.dry ? dim(`(dry run)`) : "");
  }
  error(...args: Array<unknown>) {
    this._log(red(` ERROR `), ...args, this.dry ? dim(`(dry run)`) : "");
  }
  info(...args: Array<unknown>) {
    this._log(bold(` INFO `), ...args, this.dry ? dim(`(dry run)`) : "");
  }
}
