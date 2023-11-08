#!/usr/bin/env node

import chalk from "chalk";
import { Command } from "commander";
import { logger } from "@turbo/utils";
import cliPkg from "../package.json";
import { summary, convert } from "./commands";
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
  logger.log();
  if (error instanceof ConvertError) {
    logger.log(chalk.red(error.message));
  } else {
    logger.log(chalk.red("Unexpected error. Please report it as a bug:"));
    logger.log(error);
  }
  logger.log();
  process.exit(1);
});
