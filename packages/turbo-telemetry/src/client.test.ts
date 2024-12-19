import { describe, it, mock, beforeEach } from "node:test";
import { strict as assert } from "node:assert";
import got from "got";
import { TelemetryClient } from "./client";
import { TelemetryConfig } from "./config";

describe("TelemetryClient", () => {
  beforeEach(() => {
    mock.reset();
  });

  it("sends request when batch size is reached", (t) => {
    const mockPost = mock.fn();
    t.mock.method(got, "post", mockPost);
    const config = new TelemetryConfig({
      configPath: "test-config-path",
      config: {
        telemetry_enabled: true,
        telemetry_id: "telemetry-test-id",
        telemetry_salt: "telemetry-salt",
      },
    });

    const client = new TelemetryClient({
      api: "https://example.com",
      packageInfo: {
        name: "create-turbo",
        version: "1.0.0",
      },
      config,
      opts: {
        batchSize: 2,
      },
    });

    // add two events to trigger the batch flush
    client.trackCommandStatus({
      command: "test-command",
      status: "start",
    });
    client.trackCommandStatus({
      command: "test-command",
      status: "end",
    });

    assert.equal(mockPost.mock.callCount() > 0, true);

    assert.deepEqual(
      mockPost.mock.calls[0].arguments[0],
      "https://example.com/api/turborepo/v1/events"
    );

    assert.equal(mockPost.mock.calls[0].arguments[1].json.length, 2);
    assert.deepEqual(
      Object.keys(mockPost.mock.calls[0].arguments[1].json[0].package),
      ["id", "key", "value", "package_name", "package_version", "parent_id"]
    );

    assert.equal(
      typeof mockPost.mock.calls[0].arguments[1].json[0].package.id,
      "string"
    );
    assert.equal(
      mockPost.mock.calls[0].arguments[1].json[0].package.key,
      "command:test-command"
    );
    assert.equal(
      mockPost.mock.calls[0].arguments[1].json[0].package.value,
      "start"
    );
    assert.equal(
      mockPost.mock.calls[0].arguments[1].json[0].package.package_name,
      "create-turbo"
    );
    assert.equal(
      mockPost.mock.calls[0].arguments[1].json[0].package.package_version,
      "1.0.0"
    );

    assert.deepEqual(
      Object.keys(mockPost.mock.calls[0].arguments[1].json[1].package),
      ["id", "key", "value", "package_name", "package_version", "parent_id"]
    );
    assert.equal(
      typeof mockPost.mock.calls[0].arguments[1].json[1].package.id,
      "string"
    );
    assert.equal(
      mockPost.mock.calls[0].arguments[1].json[1].package.key,
      "command:test-command"
    );
    assert.equal(
      mockPost.mock.calls[0].arguments[1].json[1].package.value,
      "end"
    );
    assert.equal(
      mockPost.mock.calls[0].arguments[1].json[1].package.package_name,
      "create-turbo"
    );
    assert.equal(
      mockPost.mock.calls[0].arguments[1].json[1].package.package_version,
      "1.0.0"
    );

    assert.equal(
      "x-turbo-session-id" in mockPost.mock.calls[0].arguments[1].headers,
      true
    );
    assert.equal(
      "x-turbo-telemetry-id" in mockPost.mock.calls[0].arguments[1].headers,
      true
    );
    assert.equal(
      /create-turbo 1\.0\.0/.test(
        mockPost.mock.calls[0].arguments[1].headers["User-Agent"]
      ),
      true
    );

    assert.equal(client.hasPendingEvents(), false);
  });

  it("does not send request before batch size is reached", (t) => {
    const mockPost = mock.fn();
    t.mock.method(got, "post", mockPost);

    const config = new TelemetryConfig({
      configPath: "test-config-path",
      config: {
        telemetry_enabled: true,
        telemetry_id: "telemetry-test-id",
        telemetry_salt: "telemetry-salt",
      },
    });

    const client = new TelemetryClient({
      api: "https://example.com",
      packageInfo: {
        name: "create-turbo",
        version: "1.0.0",
      },
      config,
    });

    client.trackCommandStatus({
      command: "test-command",
      status: "start",
    });
    assert.equal(mockPost.mock.callCount(), 0);
    assert.equal(client.hasPendingEvents(), true);
  });

  it("does not send request if telemetry is disabled", (t) => {
    const mockPost = mock.fn();
    t.mock.method(got, "post", mockPost);
    const config = new TelemetryConfig({
      configPath: "test-config-path",
      config: {
        telemetry_enabled: false,
        telemetry_id: "telemetry-test-id",
        telemetry_salt: "telemetry-salt",
      },
    });

    const client = new TelemetryClient({
      api: "https://example.com",
      packageInfo: {
        name: "create-turbo",
        version: "1.0.0",
      },
      config,
    });

    client.trackCommandStatus({
      command: "test-command",
      status: "start",
    });
    assert.equal(mockPost.mock.callCount(), 0);
    assert.equal(client.hasPendingEvents(), false);
  });

  it("flushes events when closed even if batch size is not reached", async (t) => {
    const mockPost = mock.fn((_url, _opts) => {
      // do nothing with either arg
    });
    t.mock.method(got, "post", mockPost);

    const config = new TelemetryConfig({
      configPath: "test-config-path",
      config: {
        telemetry_enabled: true,
        telemetry_id: "telemetry-test-id",
        telemetry_salt: "telemetry-salt",
      },
    });

    const client = new TelemetryClient({
      api: "https://example.com",
      packageInfo: {
        name: "create-turbo",
        version: "1.0.0",
      },
      config,
      opts: {
        batchSize: 2,
      },
    });

    // add one event
    client.trackCommandStatus({
      command: "test-command",
      status: "start",
    });

    assert.equal(mockPost.mock.callCount(), 0);

    await client.close();

    assert.equal(mockPost.mock.callCount(), 1);
    assert.equal(
      mockPost.mock.calls[0].arguments[0],
      "https://example.com/api/turborepo/v1/events"
    );

    assert.equal(mockPost.mock.calls[0].arguments[1].json.length, 1);
    assert.deepEqual(
      Object.keys(mockPost.mock.calls[0].arguments[1].json[0].package),
      ["id", "key", "value", "package_name", "package_version", "parent_id"]
    );
    assert.equal(
      typeof mockPost.mock.calls[0].arguments[1].json[0].package.id,
      "string"
    );
    assert.equal(
      mockPost.mock.calls[0].arguments[1].json[0].package.key,
      "command:test-command"
    );
    assert.equal(
      mockPost.mock.calls[0].arguments[1].json[0].package.value,
      "start"
    );
    assert.equal(
      mockPost.mock.calls[0].arguments[1].json[0].package.package_name,
      "create-turbo"
    );
    assert.equal(
      mockPost.mock.calls[0].arguments[1].json[0].package.package_version,
      "1.0.0"
    );

    assert.equal(
      "x-turbo-session-id" in mockPost.mock.calls[0].arguments[1].headers,
      true
    );
    assert.equal(
      "x-turbo-telemetry-id" in mockPost.mock.calls[0].arguments[1].headers,
      true
    );

    assert.match(
      mockPost.mock.calls[0].arguments[1].headers["User-Agent"],
      /create-turbo 1\.0\.0/
    );

    assert.equal(client.hasPendingEvents(), false);
  });
});
