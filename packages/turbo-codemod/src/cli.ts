#!/usr/bin/env node

import { red } from "picocolors";
import { logger } from "@turbo/utils";
import { Command } from "commander";
import cliPkg from "../package.json";
import { transform, migrate } from "./commands";
import { notifyUpdate } from "./utils/notifyUpdate";

const codemodCli = new Command();

codemodCli
  .name("@turbo/codemod")
  .description(
    "Codemod transformations to help upgrade your Turborepo codebase when a feature is deprecated."
  )
  .version(cliPkg.version, "-v, --version", "output the current version");

// migrate
codemodCli
  .command("migrate")
  .aliases(["update", "upgrade"])
  .description("Migrate a project to the latest version of Turborepo")
  .argument("[path]", "Directory where the transforms should be applied")
  .option(
    "--from <version>",
    "Specify the version to migrate from (default: current version)"
  )
  .option(
    "--to <version>",
    "Specify the version to migrate to (default: latest)"
  )
  .option("--install", "Install new version of turbo after migration", true)
  .option(
    "--force",
    "Bypass Git safety checks and forcibly run codemods",
    false
  )
  .option(
    "--dry, --dry-run, -d",
    "Dry run (no changes are made to files)",
    false
  )
  .option("--print", "Print transformed files to your terminal", false)
  .action(migrate);

// transform
codemodCli
  .command("transform", { isDefault: true })
  .description("Apply a single code transformation to a project")
  .argument("[transform]", "The transformer to run")
  .argument("[path]", "Directory where the transforms should be applied")
  .option(
    "--force",
    "Bypass Git safety checks and forcibly run codemods",
    false
  )
  .option("--list", "List all available transforms", false)
  .option(
    "--dry, --dry-run, -d",
    "Dry run (no changes are made to files)",
    false
  )
  .option("--print", "Print transformed files to your terminal", false)
  .action(transform);

codemodCli
  .parseAsync()
  .then(notifyUpdate)
  .catch(async (reason) => {
    logger.log();
    logger.log(red("Unexpected error. Please report it as a bug:"));
    logger.log(reason);

    logger.log();
    await notifyUpdate();
    process.exit(1);
  });
