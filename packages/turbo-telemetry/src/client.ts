import got, { type Response } from "got";
import { logger } from "@turbo/utils";
import { v4 as uuid } from "uuid";
import { buildUserAgent } from "./utils";
import { TelemetryConfig } from "./config";

const DEFAULT_BATCH_SIZE = 20;
const ENDPOINT = "/api/turborepo/v1/events";

export interface PackageInfo {
  name: string;
  version: string;
}

interface Options {
  timeout?: number;
  batchSize?: number;
}

interface Event {
  id: string;
  key: string;
  value: string;
  package_name: string;
  package_version: string;
  parent_id: string | undefined;
}

export class TelemetryClient {
  private api: string;
  private packageInfo: PackageInfo;
  private batchSize = DEFAULT_BATCH_SIZE;
  private timeout = 250;
  private sessionId = uuid();
  private eventBatches: Array<Promise<Response<string> | undefined>> = [];
  private events: Array<Record<"package", Event>> = [];

  config: TelemetryConfig;

  constructor(
    api: string,
    packageInfo: PackageInfo,
    config: TelemetryConfig,
    opts?: Options
  ) {
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
            "User-Agent": buildUserAgent(this.packageInfo),
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
  private track({
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
      id: uuid(),
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

  ////////////
  // EVENTS //
  ////////////

  /**
   * Track selected package manager
   */
  trackPackageManager(packageManager: string): Event {
    return this.track({
      key: "package_manager",
      value: packageManager,
    });
  }
}
