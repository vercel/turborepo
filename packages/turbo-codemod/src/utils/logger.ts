import picocolors from "picocolors";
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
    this._log(
      picocolors.green(" MODIFIED "),
      ...args,
      this.dry ? picocolors.dim("(dry run)") : ""
    );
  }
  unchanged(...args: Array<unknown>) {
    this._log(
      picocolors.gray(" UNCHANGED "),
      ...args,
      this.dry ? picocolors.dim("(dry run)") : ""
    );
  }
  skipped(...args: Array<unknown>) {
    this._log(
      picocolors.yellow(" SKIPPED "),
      ...args,
      this.dry ? picocolors.dim("(dry run)") : ""
    );
  }
  error(...args: Array<unknown>) {
    this._log(
      picocolors.red(" ERROR "),
      ...args,
      this.dry ? picocolors.dim("(dry run)") : ""
    );
  }
  info(...args: Array<unknown>) {
    this._log(
      picocolors.bold(" INFO "),
      ...args,
      this.dry ? picocolors.dim("(dry run)") : ""
    );
  }
}
