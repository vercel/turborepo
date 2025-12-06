#!/usr/bin/env node

import picocolors from "picocolors";
import { logger, createNotifyUpdate } from "@turbo/utils";
import { getWorkspaceDetails } from "@turbo/workspaces";
import { Command } from "commander";
import cliPkg from "../package.json";
import { transform, migrate } from "./commands";

const notifyUpdate = createNotifyUpdate({
  packageInfo: cliPkg,
  upgradeCommand: async () => {
    try {
      const { packageManager } = await getWorkspaceDetails({
        root: process.cwd(),
      });
      if (packageManager === "yarn") {
        return "yarn global add @turbo/codemod";
      } else if (packageManager === "pnpm") {
        return "pnpm i -g @turbo/codemod";
      }
      return "npm i -g @turbo/codemod";
    } catch {
      return "npm i -g @turbo/codemod";
    }
  },
});

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
  .then(() => notifyUpdate())
  .catch(async (reason) => {
    logger.log();
    logger.log(picocolors.red("Unexpected error. Please report it as a bug:"));
    logger.log(reason);

    logger.log();
    await notifyUpdate(1);
  });
