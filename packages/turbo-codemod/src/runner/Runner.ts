import chalk from "chalk";

import FileTransform from "./FileTransform";
import Logger from "../utils/logger";
import type { UtilityArgs } from "../types";
import type {
  FileResult,
  ModifyFileArgs,
  AbortTransformArgs,
  TransformerResults,
} from "./types";

class Runner {
  transform: string;
  rootPath: string;
  dry: boolean;
  print: boolean;
  modifications: Record<string, FileTransform> = {};
  logger: Logger;

  constructor(options: UtilityArgs) {
    this.transform = options.transformer;
    this.rootPath = options.rootPath;
    this.dry = options.dry;
    this.print = options.print;
    this.logger = new Logger(options);
  }

  abortTransform(args: AbortTransformArgs): TransformerResults {
    this.logger.error(args.reason);
    return {
      fatalError: new Error(args.reason),
      changes: args.changes || {},
    };
  }

  // add a file to be transformed
  modifyFile(args: ModifyFileArgs): void {
    this.modifications[args.filePath] = new FileTransform({
      rootPath: this.rootPath,
      ...args,
    });
  }

  // execute all transforms and track results for reporting
  finish(): TransformerResults {
    const results: TransformerResults = { changes: {} };
    // perform all actions and track results
    Object.keys(this.modifications).forEach((filePath) => {
      const mod = this.modifications[filePath];
      const result: FileResult = {
        action: "unchanged",
        additions: mod.additions(),
        deletions: mod.deletions(),
      };

      if (mod.hasChanges()) {
        if (this.dry) {
          result.action = "skipped";
          this.logger.skipped(chalk.dim(mod.fileName()));
        } else {
          try {
            mod.write();
            result.action = "modified";
            this.logger.modified(chalk.bold(mod.fileName()));
          } catch (err) {
            let message = "Unknown error";
            if (err instanceof Error) {
              message = err.message;
            }
            result.error = new Error(message);
            result.action = "error";
            this.logger.error(mod.fileName(), message);
          }
        }

        if (this.print) {
          mod.log({ diff: true });
        }
      } else {
        this.logger.unchanged(chalk.dim(mod.fileName()));
      }

      results.changes[mod.fileName()] = result;
    });

    const encounteredError = Object.keys(results.changes).some((fileName) => {
      return results.changes[fileName].action === "error";
    });

    if (encounteredError) {
      return this.abortTransform({
        reason: "Encountered an error while transforming files",
        changes: results.changes,
      });
    }

    return results;
  }

  static logResults(results: TransformerResults): void {
    const changedFiles = Object.keys(results.changes);
    console.log();
    if (changedFiles.length > 0) {
      console.log(chalk.bold(`Results:`));
      const table: Record<
        string,
        {
          action: FileResult["action"];
          additions: FileResult["additions"];
          deletions: FileResult["deletions"];
          error?: string;
        }
      > = {};

      changedFiles.forEach((fileName) => {
        const fileChanges = results.changes[fileName];
        table[fileName] = {
          action: fileChanges.action,
          additions: fileChanges.additions,
          deletions: fileChanges.deletions,
          error: fileChanges.error?.message || "None",
        };
      });

      console.table(table);
      console.log();
    }
  }
}

export default Runner;
