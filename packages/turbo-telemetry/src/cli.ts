import { bold, green, red } from "picocolors";
import { logger } from "@turbo/utils";
import { type Command, Argument } from "commander";
import { type TelemetryClient } from "./client";

const DEFAULT_CHOICE = "status" as const;
const CHOICES = ["enable", "disable", DEFAULT_CHOICE] as const;
type TelemetryCLIAction = (typeof CHOICES)[number];

interface TelemetryCLIOptions {
  telemetry?: TelemetryClient;
}

function status(options: TelemetryCLIOptions) {
  const isEnabled = options.telemetry?.config.isEnabled();
  logger.log(
    `Status: ${isEnabled ? bold(green("Enabled")) : bold(red("Disabled"))}`
  );
  logger.log();
  if (isEnabled) {
    logger.log(
      "Turborepo telemetry is completely anonymous. Thank you for participating!"
    );
  } else {
    logger.log(
      "You have opted-out of Turborepo anonymous telemetry. No data will be collected from your machine."
    );
  }
  logger.log("Learn more: https://turbo.build/repo/docs/telemetry");
}

function telemetry(action: TelemetryCLIAction, options: TelemetryCLIOptions) {
  if (!options.telemetry) {
    logger.error("Telemetry could not be initialized");
    return;
  }

  if (action === "enable") {
    options.telemetry.config.enable();
    logger.bold("Success!");
  } else if (action === "disable") {
    options.telemetry.config.disable();
    logger.bold("Success!");
  }

  logger.log();
  status(options);
}

/**
 * Adds a fully functional telemetry command to a CLI
 */
export function withTelemetryCommand(command: Command) {
  command
    .command("telemetry")
    .description("Manage telemetry settings")
    .addArgument(
      new Argument("[action]", "Action to perform")
        .choices(CHOICES)
        .default(DEFAULT_CHOICE)
    )
    .action(telemetry);
}
