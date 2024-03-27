import got from "got";
import { TelemetryClient } from "./client";
import { TelemetryConfig } from "./config";

jest.mock("got", () => ({
  post: jest.fn(),
}));

describe("TelemetryClient", () => {
  beforeEach(() => {
    jest.clearAllMocks();
  });

  it("sends request when batch size is reached", () => {
    const config = new TelemetryConfig({
      configPath: "test-config-path",
      config: {
        telemetry_enabled: true,
        telemetry_id: "telemetry-test-id",
        telemetry_salt: "telemetry-salt",
      },
    });

    const client = new TelemetryClient(
      "https://example.com",
      {
        name: "test-package",
        version: "1.0.0",
      },
      config,
      {
        batchSize: 2,
      }
    );

    // add two events to trigger the batch flush
    client.trackCommandStatus({
      command: "test-command",
      status: "start",
    });
    client.trackCommandStatus({
      command: "test-command",
      status: "end",
    });

    expect(got.post).toHaveBeenCalled();
    expect(got.post).toHaveBeenCalledWith(
      "https://example.com/api/turborepo/v1/events",
      expect.objectContaining({
        json: [
          {
            package: {
              id: expect.any(String) as string,
              key: "command:test-command",
              value: "start",
              package_name: "test-package",
              package_version: "1.0.0",
            },
          },
          {
            package: {
              id: expect.any(String) as string,
              key: "command:test-command",
              value: "end",
              package_name: "test-package",
              package_version: "1.0.0",
            },
          },
        ],
        headers: {
          "User-Agent": expect.stringContaining("test-package 1.0.0") as string,
          "x-turbo-session-id": expect.any(String) as string,
          "x-turbo-telemetry-id": "telemetry-test-id",
        },
      })
    );

    expect(client.hasPendingEvents()).toBe(false);
  });

  it("does not send request before batch size is reached", () => {
    const config = new TelemetryConfig({
      configPath: "test-config-path",
      config: {
        telemetry_enabled: true,
        telemetry_id: "telemetry-test-id",
        telemetry_salt: "telemetry-salt",
      },
    });

    const client = new TelemetryClient(
      "https://example.com",
      {
        name: "test-package",
        version: "1.0.0",
      },
      config
    );

    client.trackCommandStatus({
      command: "test-command",
      status: "start",
    });
    expect(got.post).not.toHaveBeenCalled();
    expect(client.hasPendingEvents()).toBe(true);
  });

  it("does not send request if telemetry is disabled", () => {
    const config = new TelemetryConfig({
      configPath: "test-config-path",
      config: {
        telemetry_enabled: false,
        telemetry_id: "telemetry-test-id",
        telemetry_salt: "telemetry-salt",
      },
    });

    const client = new TelemetryClient(
      "https://example.com",
      {
        name: "test-package",
        version: "1.0.0",
      },
      config
    );

    client.trackCommandStatus({
      command: "test-command",
      status: "start",
    });
    expect(got.post).not.toHaveBeenCalled();
    expect(client.hasPendingEvents()).toBe(false);
  });

  it("flushes events when closed even if batch size is not reached", async () => {
    const config = new TelemetryConfig({
      configPath: "test-config-path",
      config: {
        telemetry_enabled: true,
        telemetry_id: "telemetry-test-id",
        telemetry_salt: "telemetry-salt",
      },
    });

    const client = new TelemetryClient(
      "https://example.com",
      {
        name: "test-package",
        version: "1.0.0",
      },
      config,
      {
        batchSize: 2,
      }
    );

    // add one event
    client.trackCommandStatus({
      command: "test-command",
      status: "start",
    });
    expect(got.post).not.toHaveBeenCalled();

    await client.close();

    expect(got.post).toHaveBeenCalled();
    expect(got.post).toHaveBeenCalledWith(
      "https://example.com/api/turborepo/v1/events",
      expect.objectContaining({
        json: [
          {
            package: {
              id: expect.any(String) as string,
              key: "command:test-command",
              value: "start",
              package_name: "test-package",
              package_version: "1.0.0",
            },
          },
        ],
        headers: {
          "User-Agent": expect.stringContaining("test-package 1.0.0") as string,
          "x-turbo-session-id": expect.any(String) as string,
          "x-turbo-telemetry-id": "telemetry-test-id",
        },
      })
    );

    expect(client.hasPendingEvents()).toBe(false);
  });
});
