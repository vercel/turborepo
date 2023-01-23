import chalk from "chalk";
import { UtilityArgs } from "../types";

export default class Logger {
  transform: string;
  dry: boolean;

  constructor(args: UtilityArgs) {
    this.transform = args.transformer;
    this.dry = args.dry;
  }
  modified(...args: any[]) {
    console.log(
      chalk.green(` MODIFIED `),
      ...args,
      this.dry ? chalk.dim(`(dry run)`) : ""
    );
  }
  unchanged(...args: any[]) {
    console.log(
      chalk.gray(` UNCHANGED `),
      ...args,
      this.dry ? chalk.dim(`(dry run)`) : ""
    );
  }
  skipped(...args: any[]) {
    console.log(
      chalk.yellow(` SKIPPED `),
      ...args,
      this.dry ? chalk.dim(`(dry run)`) : ""
    );
  }
  error(...args: any[]) {
    console.log(
      chalk.red(` ERROR `),
      ...args,
      this.dry ? chalk.dim(`(dry run)`) : ""
    );
  }
  info(...args: any[]) {
    console.log(
      chalk.bold(` INFO `),
      ...args,
      this.dry ? chalk.dim(`(dry run)`) : ""
    );
  }
}
