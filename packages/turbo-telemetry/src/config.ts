import { readFileSync, writeFileSync } from "node:fs";
import { logger } from "@turbo/utils";
import chalk from "chalk";
import { defaultConfigPath } from "./utils";

const DEBUG_ENV_VAR = "TURBO_TELEMETRY_DEBUG";
const DISABLED_ENV_VAR = "TURBO_TELEMETRY_DISABLED";
const DISABLED_MESSAGE_ENV_VAR = "TURBO_TELEMETRY_MESSAGE_DISABLED";
const DO_NOT_TRACK_ENV_VAR = "DO_NOT_TRACK";

interface Config {
  telemetry_enabled: boolean;
  telemetry_id: string;
  telemetry_salt: string;
  telemetry_alerted?: Date;
}

/**
 * NOTE: This package is a direct port of the telemetry config struct from the turbo-telemetry crate. Any changes
 * made here should be reflected in the turbo-telemetry crate as well.
 *
 * https://github.com/vercel/turbo/blob/main/crates/turborepo-telemetry/src/config.rs
 */
export class TelemetryConfig {
  private config: Config;
  private configPath: string;

  constructor({ configPath, config }: { configPath: string; config: Config }) {
    this.config = config;
    this.configPath = configPath;
  }

  static async fromDefaultConfig() {
    const configPath = await defaultConfigPath();
    const file = readFileSync(configPath, "utf-8");
    const config = JSON.parse(file) as Config;
    return new TelemetryConfig({ configPath, config });
  }

  write() {
    const json = JSON.stringify(this.config, null, 2);
    writeFileSync(this.configPath, json);
  }

  hasSeenAlert(): boolean {
    return this.config.telemetry_alerted !== undefined;
  }

  isEnabled(): boolean {
    const doNotTrack = process.env[DO_NOT_TRACK_ENV_VAR] || "0";
    const turboTelemetryDisabled = process.env[DISABLED_ENV_VAR] || "0";

    if (
      doNotTrack === "1" ||
      doNotTrack.toLowerCase() === "true" ||
      turboTelemetryDisabled === "1" ||
      turboTelemetryDisabled.toLowerCase() === "true"
    ) {
      return false;
    }

    return this.config.telemetry_enabled;
  }

  isTelemetryWarningEnabled(): boolean {
    const turboTelemetryMsgDisabled =
      process.env[DISABLED_MESSAGE_ENV_VAR] || "0";

    const isDisabled =
      turboTelemetryMsgDisabled === "1" ||
      turboTelemetryMsgDisabled.toLowerCase() === "true";

    return !isDisabled;
  }

  get id() {
    return this.config.telemetry_id;
  }

  showAlert() {
    if (
      !this.hasSeenAlert() &&
      this.isEnabled() &&
      this.isTelemetryWarningEnabled()
    ) {
      logger.log();
      logger.bold("Attention:");
      logger.grey(
        "Turborepo now collects completely anonymous telemetry regarding usage."
      );
      logger.grey(
        "This information is used to shape the Turborepo roadmap and prioritize features."
      );
      logger.grey(
        "You can learn more, including how to opt-out if you'd not like to participate in this anonymous program, by visiting the following URL:"
      );
      logger.underline(chalk.grey("https://turbo.build/repo/docs/telemetry"));
    }

    this.alertShown();
  }

  enable() {
    this.config.telemetry_enabled = true;
    this.write();
  }

  disable() {
    this.config.telemetry_enabled = false;
    this.write();
  }

  alertShown() {
    if (this.hasSeenAlert()) {
      return true;
    }

    this.config.telemetry_alerted = new Date();
    this.write();
    return true;
  }

  static isDebug() {
    const debug = process.env[DEBUG_ENV_VAR] || "0";
    return debug === "1" || debug.toLowerCase() === "true";
  }
}
