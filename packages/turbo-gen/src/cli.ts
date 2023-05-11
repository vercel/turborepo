#!/usr/bin/env node

import chalk from "chalk";
import { Argument, Command, Option } from "commander";
import notifyUpdate from "./utils/notifyUpdate";
import { logger } from "@turbo/utils";

import { add, generate, raw } from "./commands";
import cliPkg from "../package.json";

const turboGenCli = new Command();

turboGenCli
  .name(chalk.bold(logger.turboGradient("@turbo/gen")))
  .description("Extend your Turborepo")
  .version(cliPkg.version, "-v, --version", "Output the current version")
  .helpOption("-h, --help", "Display help for command")
  .showHelpAfterError(false);

turboGenCli
  .command("raw", { hidden: true })
  .argument("<type>", "The type of generator to run")
  .addOption(new Option("--json <arguments>", "Arguments as raw JSON"))
  .action(raw);

turboGenCli
  .command("add")
  .aliases(["a"])
  .description("Add a new package or app to your project")
  .addOption(
    new Option("-n, --name <workspace-name>", "Name for the new workspace")
  )
  .addOption(
    new Option("-b, --empty", "Generate an empty workspace")
      .conflicts("copy")
      .default(true)
  )
  .addOption(
    new Option(
      "-c, --copy",
      "Generate a workspace using an existing workspace as a template"
    ).conflicts("empty")
  )
  .addOption(
    new Option(
      "-d, --destination <dir>",
      "Where the new workspace should be created"
    )
  )
  .addOption(
    new Option("-w, --what <type>", "The type of workspace to create").choices([
      "app",
      "package",
    ])
  )
  .addOption(
    new Option(
      "-r, --root <dir>",
      "The root of your repository (default: directory with root turbo.json)"
    )
  )
  .addOption(
    new Option(
      "-e, --example [github-url]",
      `An example package to add. You can use a GitHub URL with any branch and/or subdirectory.`
    ).implies({ copy: true })
  )
  .addOption(
    new Option(
      "-p, --example-path <path-to-example>",
      `In a rare case, your GitHub URL might contain a branch name with
a slash (e.g. bug/fix-1) and the path to the example (e.g. foo/bar).
In this case, you must specify the path to the example separately:
--example-path foo/bar
`
    ).implies({ copy: true })
  )
  .addOption(
    new Option(
      "--show-all-dependencies",
      "Do not filter available dependencies by the workspace type"
    ).default(false)
  )
  .action(add);

turboGenCli
  .command("generate")
  .aliases(["g", "gen"])
  .description("Run custom generators")
  .addArgument(
    new Argument("[generator-name]", "The name of the generator to run")
  )
  .addOption(
    new Option(
      "-c, --config <config>",
      "Generator configuration file (default: turbo/generators/config.js"
    )
  )
  .addOption(
    new Option(
      "-r, --root <dir>",
      "The root of your repository (default: directory with root turbo.json)"
    )
  )
  .addOption(
    new Option(
      "-a, --args <args...>",
      "Arguments passed directly to generator"
    ).default([])
  )
  .action(generate);

turboGenCli
  .parseAsync()
  .then(notifyUpdate)
  .catch(async (reason) => {
    console.log();
    if (reason.command) {
      logger.error(`${chalk.bold(reason.command)} has failed.`);
    } else {
      logger.error("Unexpected error. Please report it as a bug:");
      console.log(reason);
    }
    console.log();
    await notifyUpdate();
    process.exit(1);
  });
