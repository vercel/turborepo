import { randomUUID } from "node:crypto";
import got, { type Response } from "got";
import { logger } from "@turbo/utils";
import utils from "./utils";
import { TelemetryConfig } from "./config";
import type { Event, PackageInfo } from "./events/types";

const DEFAULT_BATCH_SIZE = 20;
const ENDPOINT = "/api/turborepo/v1/events";

export interface Args {
  api: string;
  packageInfo: PackageInfo;
  config: TelemetryConfig;
  opts?: Options;
}

interface Options {
  timeout?: number;
  batchSize?: number;
}

export class TelemetryClient {
  private api: string;
  private packageInfo: PackageInfo;
  private batchSize = DEFAULT_BATCH_SIZE;
  private timeout = 250;
  private sessionId = randomUUID();
  private eventBatches: Array<Promise<Response<string> | undefined>> = [];
  private events: Array<Record<"package", Event>> = [];

  config: TelemetryConfig;

  constructor({ api, packageInfo, config, opts }: Args) {
    // build the telemetry api url with the given base
    const telemetryApi = new URL(ENDPOINT, api);
    this.api = telemetryApi.toString();
    this.packageInfo = packageInfo;
    this.config = config;

    if (opts?.timeout) {
      this.timeout = opts.timeout;
    }
    if (opts?.batchSize) {
      this.batchSize = opts.batchSize;
    }
  }

  hasPendingEvents(): boolean {
    return this.events.length !== 0;
  }

  async waitForFlush(): Promise<void> {
    await Promise.all(this.eventBatches);
  }

  /**
   * Flushes the telemetry events by sending them to the server.
   */
  private flushEvents() {
    const batch = this.events.splice(0, this.batchSize);

    if (this.config.isEnabled()) {
      // track the promises on the class
      this.eventBatches.push(
        got.post(this.api, {
          timeout: this.timeout,
          json: batch,
          headers: {
            "x-turbo-telemetry-id": this.config.id,
            "x-turbo-session-id": this.sessionId,
            "User-Agent": utils.buildUserAgent(this.packageInfo),
          },
        })
      );
    }
  }

  /**
   * Method that tracks the given key value pair.
   *
   * NOTE: This is intentionally private to prevent misuse.
   * All tracking should be done through the public methods.
   * If a new event is needed, a new public method should be created.
   */
  protected track({
    key,
    value,
    parentId,
    isSensitive,
  }: {
    key: string;
    value: string;
    parentId?: string;
    isSensitive?: boolean;
  }): Event {
    const event = {
      id: randomUUID(),
      key,
      value: isSensitive ? this.config.oneWayHash(value) : value,
      package_name: this.packageInfo.name,
      package_version: this.packageInfo.version,
      parent_id: parentId,
    };

    if (TelemetryConfig.isDebug()) {
      logger.log();
      logger.bold("[telemetry event]");
      logger.dimmed(JSON.stringify(event, null, 2));
      logger.log();
    }

    if (this.config.isEnabled()) {
      this.events.push({ package: event });

      // flush if we have enough events
      if (this.events.length >= this.batchSize) {
        this.flushEvents();
      }
    }

    return event;
  }

  /**
   * Closes the client and flushes any pending requests.
   */
  async close(): Promise<void> {
    while (this.hasPendingEvents()) {
      this.flushEvents();
    }
    try {
      await this.waitForFlush();
    } catch (err) {
      // fail silently if we can't send telemetry
    }
  }

  protected trackCliOption({
    option,
    value,
  }: {
    option: string;
    value: string;
  }): Event {
    return this.track({
      key: `option:${option}`,
      value,
    });
  }

  protected trackCliArgument({
    argument,
    value,
  }: {
    argument: string;
    value: string;
  }): Event {
    return this.track({
      key: `argument:${argument}`,
      value,
    });
  }

  protected trackCliCommand({
    command,
    value,
  }: {
    command: string;
    value: string;
  }): Event {
    return this.track({
      key: `command:${command}`,
      value,
    });
  }

  ///////////////////
  // SHARED EVENTS //
  //////////////////

  trackCommandStatus({
    command,
    status,
  }: {
    command: string;
    status: string;
  }): Event {
    return this.trackCliCommand({
      command,
      value: status,
    });
  }

  trackCommandWarning(warning: string): Event | undefined {
    return this.track({
      key: "warning",
      value: warning,
    });
  }

  trackCommandError(error: string): Event | undefined {
    return this.track({
      key: "error",
      value: error,
    });
  }
}
