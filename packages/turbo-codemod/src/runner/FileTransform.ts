import chalk from "chalk";
import { diffLines, Change, diffJson } from "diff";
import fs from "fs-extra";
import os from "os";
import path from "path";

import type { FileTransformArgs, LogFileArgs } from "./types";

export default class FileTransform {
  filePath: string;
  rootPath: string;
  before: string | object;
  after?: string | object;
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
          this.before = fs.readJsonSync(args.filePath);
        } else {
          this.before = fs.readFileSync(args.filePath);
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
        fs.writeJsonSync(this.filePath, this.after, { spaces: 2 });
      } else {
        fs.writeFileSync(this.filePath, this.after);
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
      console.log(os.EOL);
    } else {
      console.log(this.after);
    }
  }
}
