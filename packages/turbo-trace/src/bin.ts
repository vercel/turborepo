#!/usr/bin/env node
import { readFileSync } from "node:fs";
import { hasWork } from "./selection";

// Prints the directories (arguments, or the lines of stdin) in which a test
// file would run, one on each line.
const args = process.argv.slice(2);
const dirs = args.length > 0 ? args : readFileSync(0, "utf8").split(/\r?\n/);
for (const dir of dirs.filter((dir) => dir !== "" && hasWork(dir))) {
  process.stdout.write(`${dir}\n`);
}
