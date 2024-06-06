#!/usr/bin/env node

import { Command, Option } from "commander";
import {
  type TurboIgnoreTelemetry,
  initTelemetry,
  withTelemetryCommand,
} from "@turbo/telemetry";
import cliPkg from "../package.json";
import { turboIgnore } from "./ignore";

// Global telemetry client
let telemetryClient: TurboIgnoreTelemetry | undefined;

const turboIgnoreCli = new Command();

turboIgnoreCli
  .name(cliPkg.name)
  .description(
    "Only proceed with deployment if the workspace or any of its dependencies have changed"
  )
  .hook("preAction", async (_, thisAction) => {
    const { telemetry } = await initTelemetry<"turbo-ignore">({
      packageInfo: {
        name: "turbo-ignore",
        version: cliPkg.version,
      },
    });
    // inject telemetry into the action as an option
    thisAction.addOption(
      new Option("--telemetry").default(telemetry).hideHelp()
    );
    telemetryClient = telemetry;
  })
  .hook("postAction", async () => {
    await telemetryClient?.close();
  })
  .argument(
    "[workspace]",
    `The workspace being deployed. If [workspace] is not provided, it will be inferred from the "name" field of the "package.json" located at the current working directory.`
  )
  .addOption(
    new Option("-t, --task <name>", "The task to execute").default("build")
  )
  .addOption(
    new Option(
      "-f, --fallback <ref>",
      "On Vercel, if no previously deployed SHA is available to compare against, fallback to comparing against the provided ref"
    )
  )
  .addOption(
    new Option(
      "-d, --directory <path>",
      "The directory to run in (default: cwd)"
    )
  )
  .addOption(
    new Option(
      "--turbo-version <version>",
      "Explicitly set which version of turbo to invoke"
    )
  )
  .addOption(
    new Option(
      "-b, --max-buffer <number>",
      "maxBuffer for the child process in KB (default: 1024 KB)"
    ).argParser((val) => parseInt(val, 10) * 1024)
  )
  .version(cliPkg.version, "-v, --version", "Output the current version")
  .helpOption("-h, --help", "Display help for command")
  .showHelpAfterError(false)
  .action(turboIgnore);

// Add telemetry command to the CLI
withTelemetryCommand(turboIgnoreCli);

turboIgnoreCli.parse();
