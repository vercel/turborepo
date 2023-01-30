#!/usr/bin/env node

import chalk from "chalk";
import { Command } from "commander";

import { transform, migrate } from "./commands";
import notifyUpdate from "./utils/notifyUpdate";
import cliPkg from "../package.json";

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
  .option("--dry", "Dry run (no changes are made to files)", false)
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
  .option("--dry", "Dry run (no changes are made to files)", false)
  .option("--print", "Print transformed files to your terminal", false)
  .action(transform);

codemodCli
  .parseAsync()
  .then(notifyUpdate)
  .catch(async (reason) => {
    console.log();
    if (reason.command) {
      console.log(`  ${chalk.cyan(reason.command)} has failed.`);
    } else {
      console.log(chalk.red("Unexpected error. Please report it as a bug:"));
      console.log(reason);
    }
    console.log();
    await notifyUpdate();
    process.exit(1);
  });
