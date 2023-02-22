#!/usr/bin/env node

import chalk from "chalk";
import { Command } from "commander";

import { summary, convert } from "./commands";
import cliPkg from "../package.json";
import { ConvertError } from "./errors";

const workspacesCli = new Command();

workspacesCli
  .name("@turbo/workspaces")
  .description("Tools for working with package manager workspaces")
  .version(cliPkg.version, "-v, --version", "output the current version");

// convert
workspacesCli
  .command("convert")
  .description("Convert project between workspace managers")
  .argument("[path]", "Project root")
  .argument("[package-manager]", "Package manager to convert to")
  .option(
    "--skip-install",
    "Do not run a package manager install after conversion",
    false
  )
  .option("--dry", "Dry run (no changes are made to files)", false)
  .option(
    "--force",
    "Bypass Git safety checks and forcibly run conversion",
    false
  )
  .action(convert);

// summary
workspacesCli
  .command("summary")
  .description("Display a summary of the specified project")
  .argument("[path]", "Project root")
  .action(summary);

workspacesCli.parseAsync().catch((error) => {
  console.log();
  if (error instanceof ConvertError) {
    console.log(chalk.red(error.message));
  } else {
    console.log(chalk.red("Unexpected error. Please report it as a bug:"));
    console.log(error.message);
  }
  console.log();
  process.exit(1);
});
