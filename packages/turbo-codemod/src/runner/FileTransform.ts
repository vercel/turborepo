import os from "node:os";
import path from "node:path";
import { logger } from "@turbo/utils";
import chalk from "chalk";
import type { Change } from "diff";
import { diffLines, diffJson } from "diff";
import {
  readJsonSync,
  readFileSync,
  writeJsonSync,
  writeFileSync,
} from "fs-extra";
import type { FileTransformArgs, LogFileArgs } from "./types";

export class FileTransform {
  filePath: string;
  rootPath: string;
  before: string | object;
  after?: string | object | null;
  error?: Error;
  changes: Array<Change> = [];

  constructor(args: FileTransformArgs) {
    this.filePath = args.filePath;
    this.rootPath = args.rootPath;
    this.after = args.after;
    this.error = args.error;

    // load original file for comparison
    if (args.before === undefined) {
      try {
        if (path.extname(args.filePath) === ".json") {
          this.before = readJsonSync(args.filePath) as object;
        } else {
          this.before = readFileSync(args.filePath);
        }
      } catch (err) {
        this.before = "";
      }
    } else if (args.before === null) {
      this.before = "";
    } else {
      this.before = args.before;
    }

    // determine diff
    if (args.after) {
      if (typeof this.before === "object" || typeof args.after === "object") {
        this.changes = diffJson(this.before, args.after);
      } else {
        this.changes = diffLines(this.before, args.after);
      }
    } else {
      this.changes = [];
    }
  }

  fileName(): string {
    return path.relative(this.rootPath, this.filePath);
  }

  write(): void {
    if (this.after) {
      if (typeof this.after === "object") {
        writeJsonSync(this.filePath, this.after, { spaces: 2 });
      } else {
        writeFileSync(this.filePath, this.after);
      }
    }
  }

  additions(): number {
    return this.changes.filter((c) => c.added).length;
  }

  deletions(): number {
    return this.changes.filter((c) => c.removed).length;
  }

  hasChanges(): boolean {
    return this.additions() > 0 || this.deletions() > 0;
  }

  log(args: LogFileArgs): void {
    if (args.diff) {
      this.changes.forEach((part) => {
        if (part.added) {
          process.stdout.write(chalk.green(part.value));
        } else if (part.removed) {
          process.stdout.write(chalk.red(part.value));
        } else {
          process.stdout.write(chalk.dim(part.value));
        }
      });
      logger.log(os.EOL);
    } else {
      logger.log(this.after);
    }
  }
}
