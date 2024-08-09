import { readFileSync, writeFileSync, rmSync } from "node:fs";
import { randomUUID } from "node:crypto";
import { logger } from "@turbo/utils";
import { gray } from "picocolors";
import { z } from "zod";
import utils from "./utils";

const DEBUG_ENV_VAR = "TURBO_TELEMETRY_DEBUG";
const DISABLED_ENV_VAR = "TURBO_TELEMETRY_DISABLED";
const DISABLED_MESSAGE_ENV_VAR = "TURBO_TELEMETRY_MESSAGE_DISABLED";
const DO_NOT_TRACK_ENV_VAR = "DO_NOT_TRACK";

const ConfigSchema = z.object({
  telemetry_enabled: z.boolean(),
  telemetry_id: z.string(),
  telemetry_salt: z.string(),
  telemetry_alerted: z.string().optional(),
});

type Config = z.infer<typeof ConfigSchema>;

/**
 * NOTE: This package is a direct port of the telemetry config struct from the turbo-telemetry crate. Any changes
 * made here should be reflected in the turbo-telemetry crate as well.
 *
 * https://github.com/vercel/turborepo/blob/main/crates/turborepo-telemetry/src/config.rs
 */
export class TelemetryConfig {
  config: Config;
  private configPath: string;

  constructor({ configPath, config }: { configPath: string; config: Config }) {
    this.config = config;
    this.configPath = configPath;
  }

  static fromConfigPath(configPath: string): TelemetryConfig | undefined {
    try {
      const file = readFileSync(configPath, "utf-8");
      const rawConfig = JSON.parse(file) as unknown;
      const config = TelemetryConfig.validateConfig(rawConfig);
      return new TelemetryConfig({ configPath, config });
    } catch (e) {
      if (TelemetryConfig.tryRemove({ configPath })) {
        return TelemetryConfig.create({ configPath });
      }

      return undefined;
    }
  }

  static async fromDefaultConfig(): Promise<TelemetryConfig | undefined> {
    try {
      const configPath = await utils.defaultConfigPath();
      return TelemetryConfig.fromConfigPath(configPath);
    } catch (e) {
      return undefined;
    }
  }

  static validateConfig(config: unknown): Config {
    try {
      return ConfigSchema.parse(config);
    } catch (e) {
      throw new Error("Config is invalid.");
    }
  }

  static create({
    configPath,
  }: {
    configPath: string;
  }): TelemetryConfig | undefined {
    const RawTelemetryId = randomUUID();
    const telemetrySalt = randomUUID();
    const telemetryId = utils.oneWayHashWithSalt({
      input: RawTelemetryId,
      salt: telemetrySalt,
    });

    const config = new TelemetryConfig({
      configPath,
      config: {
        telemetry_enabled: true,
        telemetry_id: telemetryId,
        telemetry_salt: telemetrySalt,
      },
    });

    const saved = config.tryWrite();
    if (saved) {
      return config;
    }
    return undefined;
  }

  tryWrite(): boolean {
    try {
      const json = JSON.stringify(this.config, null, 2);
      writeFileSync(this.configPath, json);
      return true;
    } catch (e) {
      return false;
    }
  }

  static tryRemove({ configPath }: { configPath: string }): boolean {
    try {
      rmSync(configPath, {
        force: true,
      });
      return true;
    } catch (e) {
      return false;
    }
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

  showAlert(): void {
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
      logger.underline(gray("https://turbo.build/repo/docs/telemetry"));
    }

    this.alertShown();
  }

  enable(): void {
    this.config.telemetry_enabled = true;
    this.tryWrite();
  }

  disable(): void {
    this.config.telemetry_enabled = false;
    this.tryWrite();
  }

  alertShown(): boolean {
    if (this.hasSeenAlert()) {
      return true;
    }

    this.config.telemetry_alerted = new Date().toISOString();
    this.tryWrite();
    return true;
  }

  oneWayHash(input: string): string {
    return utils.oneWayHashWithSalt({
      input,
      salt: this.config.telemetry_salt,
    });
  }

  static isDebug(): boolean {
    const debug = process.env[DEBUG_ENV_VAR] || "0";
    return debug === "1" || debug.toLowerCase() === "true";
  }
}
