import { info } from "./logger";
import { TurboIgnoreArgs } from "./types";

export function getTask(args: TurboIgnoreArgs): string | null {
  if (args.task) {
    info(`using "${args.task}" as the task from the arguments`);
    return `"${args.task}"`;
  }

  info('using "build" as the task as it was unspecified');

  return "build";
}
