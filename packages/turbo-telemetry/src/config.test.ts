import { describe, test, mock, afterEach, beforeEach } from "node:test";
import { strict as assert } from "node:assert";
import fs from "node:fs";
import { TelemetryConfig } from "./config";
import utils from "./utils";

describe("TelemetryConfig", () => {
  let telemetryConfig: TelemetryConfig;

  beforeEach(() => {
    telemetryConfig = new TelemetryConfig({
      configPath: "/path/to/config.json",
      config: {
        telemetry_enabled: true,
        telemetry_id: "123456",
        telemetry_salt: "private-salt",
      },
    });
  });

  afterEach(() => {
    mock.reset();

    delete process.env.DO_NOT_TRACK;
    delete process.env.TURBO_TELEMETRY_DISABLED;
    delete process.env.TURBO_TELEMETRY_MESSAGE_DISABLED;
    delete process.env.TURBO_TELEMETRY_DEBUG;
  });

  describe("fromDefaultConfig", () => {
    test("should create TelemetryConfig instance from default config", async (t) => {
      const mockConfigPath = "/path/to/defaultConfig.json";
      const mockFileContent = JSON.stringify({
        telemetry_enabled: true,
        telemetry_id: "654321",
        telemetry_salt: "default-salt",
      });

      const mockDefaultConfigPath = mock.fn(() => mockConfigPath);
      const mockReadFileSync = mock.fn(() => mockFileContent);

      t.mock.method(utils, "defaultConfigPath", mockDefaultConfigPath);
      t.mock.method(fs, "readFileSync", mockReadFileSync);

      const result = await TelemetryConfig.fromDefaultConfig();

      assert.equal(mockDefaultConfigPath.mock.calls.length > 0, true);
      assert.deepEqual(mockReadFileSync.mock.calls[0].arguments, [
        mockConfigPath,
        "utf-8",
      ]);
      assert.equal(result instanceof TelemetryConfig, true);
      assert.equal(result?.id, "654321");
    });

    test("should generate new config if default config doesn't exist", async (t) => {
      const mockConfigPath = "/path/to/defaultConfig.json";
      const mockDefaultConfigPath = mock.fn(() => mockConfigPath);
      const mockReadFileSync = mock.fn(() => {
        throw new Error("File not found");
      });
      const mockRmSync = mock.fn();
      const mockWriteFileSync = mock.fn();

      t.mock.method(utils, "defaultConfigPath", mockDefaultConfigPath);
      t.mock.method(fs, "readFileSync", mockReadFileSync);
      t.mock.method(fs, "rmSync", mockRmSync);
      t.mock.method(fs, "writeFileSync", mockWriteFileSync);

      const result = await TelemetryConfig.fromDefaultConfig();

      assert.equal(mockDefaultConfigPath.mock.calls.length > 0, true);
      assert.deepEqual(mockReadFileSync.mock.calls[0].arguments, [
        mockConfigPath,
        "utf-8",
      ]);
      assert.equal(mockRmSync.mock.calls.length, 1);
      assert.deepEqual(mockRmSync.mock.calls[0].arguments, [
        mockConfigPath,
        {
          force: true,
        },
      ]);

      assert.equal(mockWriteFileSync.mock.calls.length, 1);

      assert.deepEqual(
        mockWriteFileSync.mock.calls[0].arguments[0],
        mockConfigPath
      );

      // eslint-disable-next-line @typescript-eslint/no-unsafe-assignment -- types are wrong?
      const parsedSecondArg = JSON.parse(
        mockWriteFileSync.mock.calls[0].arguments[1]
      );
      assert.deepEqual(parsedSecondArg.telemetry_enabled, true);
      assert.deepEqual(typeof parsedSecondArg.telemetry_id, "string");
      assert.deepEqual(typeof parsedSecondArg.telemetry_salt, "string");

      assert.equal(result instanceof TelemetryConfig, true);
      assert.equal(typeof result?.id, "string");
      assert.equal(result?.config.telemetry_enabled, true);
    });

    test("should not throw if default config is missing a key", async (t) => {
      const mockConfigPath = "/path/to/defaultConfig.json";
      const id = "654321";
      const mockFileContent = JSON.stringify({
        // missing telemetry_enabled
        telemetry_id: id,
        telemetry_salt: "default-salt",
      });
      const mockRmSync = mock.fn();
      const mockWriteFileSync = mock.fn();

      const mockDefaultConfigPath = mock.fn(() => mockConfigPath);
      const mockReadFileSync = mock.fn(() => mockFileContent);

      t.mock.method(utils, "defaultConfigPath", mockDefaultConfigPath);
      t.mock.method(fs, "readFileSync", mockReadFileSync);
      t.mock.method(fs, "rmSync", mockRmSync);
      t.mock.method(fs, "writeFileSync", mockWriteFileSync);

      const result = await TelemetryConfig.fromDefaultConfig();

      assert.equal(mockDefaultConfigPath.mock.calls.length, 1);
      assert.deepEqual(mockReadFileSync.mock.calls[0].arguments, [
        mockConfigPath,
        "utf-8",
      ]);
      assert.equal(mockRmSync.mock.calls.length, 1);
      assert.deepEqual(mockRmSync.mock.calls[0].arguments, [
        mockConfigPath,
        {
          force: true,
        },
      ]);
      assert.equal(mockWriteFileSync.mock.calls.length, 1);
      assert.deepEqual(
        mockWriteFileSync.mock.calls[0].arguments[0],
        mockConfigPath
      );
      assert.equal(
        typeof mockWriteFileSync.mock.calls[0].arguments[1],
        "string"
      );

      assert.equal(result instanceof TelemetryConfig, true);
      assert.equal(typeof result?.id, "string");
      // this shouldn't match because we threw away the file and made a new one
      assert.notEqual(result?.id, id);
      assert.equal(result?.config.telemetry_enabled, true);
    });

    test("should not throw if default config has a key of the wrong type", async (t) => {
      const mockConfigPath = "/path/to/defaultConfig.json";
      const salt = "default-salt";
      const mockFileContent = JSON.stringify({
        telemetry_enabled: true,
        // telemetry_id should be a string
        telemetry_id: true,
        telemetry_salt: salt,
      });
      const mockRmSync = mock.fn();
      const mockWriteFileSync = mock.fn();

      const mockDefaultConfigPath = mock.fn(() => mockConfigPath);
      const mockReadFileSync = mock.fn(() => mockFileContent);

      t.mock.method(utils, "defaultConfigPath", mockDefaultConfigPath);
      t.mock.method(fs, "readFileSync", mockReadFileSync);
      t.mock.method(fs, "rmSync", mockRmSync);
      t.mock.method(fs, "writeFileSync", mockWriteFileSync);

      const result = await TelemetryConfig.fromDefaultConfig();

      assert.equal(mockDefaultConfigPath.mock.calls.length, 1);
      assert.deepEqual(mockReadFileSync.mock.calls[0].arguments, [
        mockConfigPath,
        "utf-8",
      ]);
      assert.equal(mockRmSync.mock.calls.length, 1);
      assert.deepEqual(mockRmSync.mock.calls[0].arguments, [
        mockConfigPath,
        {
          force: true,
        },
      ]);
      assert.equal(mockWriteFileSync.mock.calls.length, 1);
      assert.equal(
        mockWriteFileSync.mock.calls[0].arguments[0],
        mockConfigPath
      );
      assert.equal(
        typeof mockWriteFileSync.mock.calls[0].arguments[1],
        "string"
      );
      assert.equal(result instanceof TelemetryConfig, true);
      assert.equal(typeof result?.id, "string");
      // this shouldn't match because we threw away the file and made a new one
      assert.notEqual(result?.config.telemetry_salt, salt);
      assert.equal(result?.config.telemetry_enabled, true);
    });
  });

  describe("write", () => {
    test("should write the config to the file", (t) => {
      const mockWriteFileSync = mock.fn();
      t.mock.method(fs, "writeFileSync", mockWriteFileSync);

      const mockJson = JSON.stringify(telemetryConfig.config, null, 2);
      telemetryConfig.tryWrite();

      assert.deepEqual(mockWriteFileSync.mock.calls[0].arguments, [
        "/path/to/config.json",
        mockJson,
      ]);
    });

    test("should not throw if write fails", (t) => {
      const mockWriteFileSync = t.mock.method(fs, "writeFileSync", () => {
        throw new Error("Write error");
      });

      const mockJson = JSON.stringify(telemetryConfig.config, null, 2);
      // this shouldn't throw
      telemetryConfig.tryWrite();
      assert.deepStrictEqual(mockWriteFileSync.mock.calls[0].arguments, [
        "/path/to/config.json",
        mockJson,
      ]);
    });
  });

  describe("hasSeenAlert", () => {
    test("should return true if telemetry_alerted is defined", () => {
      telemetryConfig = new TelemetryConfig({
        configPath: "/path/to/config.json",
        config: {
          telemetry_enabled: true,
          telemetry_id: "123456",
          telemetry_salt: "private-salt",
          telemetry_alerted: new Date().toISOString(),
        },
      });

      const result = telemetryConfig.hasSeenAlert();

      assert.equal(result, true);
    });

    test("should return false if telemetry_alerted key exists but is undefined", () => {
      telemetryConfig = new TelemetryConfig({
        configPath: "/path/to/config.json",
        config: {
          telemetry_enabled: true,
          telemetry_id: "123456",
          telemetry_salt: "private-salt",
          telemetry_alerted: undefined,
        },
      });
      const result = telemetryConfig.hasSeenAlert();

      assert.equal(result, false);
    });

    test("should return false if telemetry_alerted is undefined", () => {
      const result = telemetryConfig.hasSeenAlert();

      assert.equal(result, false);
    });
  });

  describe("isEnabled", () => {
    const testCases = [
      { envVar: "DO_NOT_TRACK", value: "1", expectedResult: false },
      { envVar: "DO_NOT_TRACK", value: "true", expectedResult: false },
      { envVar: "TURBO_TELEMETRY_DISABLED", value: "1", expectedResult: false },
      {
        envVar: "TURBO_TELEMETRY_DISABLED",
        value: "true",
        expectedResult: false,
      },
      { envVar: null, value: null, expectedResult: true },
    ];
    for (const { envVar, value, expectedResult } of testCases) {
      test(`should return ${expectedResult} when ${envVar} is set to '${value}'`, () => {
        const config = new TelemetryConfig({
          configPath: "/path/to/config.json",
          config: {
            telemetry_enabled: true,
            telemetry_id: "123456",
            telemetry_salt: "private-salt",
          },
        });

        if (envVar) {
          process.env[envVar] = value;
        }

        const result = config.isEnabled();
        assert.equal(result, expectedResult);
      });
    }
  });

  describe("isTelemetryWarningEnabled", () => {
    test("should return false if TURBO_TELEMETRY_MESSAGE_DISABLED is set to '1'", () => {
      process.env.TURBO_TELEMETRY_MESSAGE_DISABLED = "1";

      const result = telemetryConfig.isTelemetryWarningEnabled();

      assert.equal(result, false);
    });

    test("should return false if TURBO_TELEMETRY_MESSAGE_DISABLED is set to 'true'", () => {
      process.env.TURBO_TELEMETRY_MESSAGE_DISABLED = "true";

      const result = telemetryConfig.isTelemetryWarningEnabled();

      assert.equal(result, false);
    });

    test("should return true if TURBO_TELEMETRY_MESSAGE_DISABLED is not set", () => {
      const result = telemetryConfig.isTelemetryWarningEnabled();

      assert.equal(result, true);
    });
  });

  describe("showAlert", () => {
    test("should log the telemetry alert if conditions are met", (t) => {
      const mockLog = t.mock.method(console, "log");
      telemetryConfig.showAlert();
      assert.equal(mockLog.mock.calls.length, 6);
    });

    test("should not log the telemetry alert if conditions are not met", (t) => {
      const mockLog = t.mock.method(console, "log");

      telemetryConfig = new TelemetryConfig({
        configPath: "/path/to/config.json",
        config: {
          telemetry_enabled: false,
          telemetry_id: "123456",
          telemetry_salt: "private-salt",
        },
      });

      telemetryConfig.showAlert();

      assert.deepEqual(mockLog.mock.calls.length, 0);
    });
  });

  describe("enable", () => {
    test("should set telemetry_enabled to true and write the config", (t) => {
      const mockWriteFileSync = t.mock.method(fs, "writeFileSync");

      telemetryConfig.enable();
      assert.equal(telemetryConfig.isEnabled(), true);
      assert.equal(mockWriteFileSync.mock.calls.length, 1);
      assert.deepStrictEqual(mockWriteFileSync.mock.calls[0].arguments, [
        "/path/to/config.json",
        JSON.stringify(telemetryConfig.config, null, 2),
      ]);
    });
  });

  describe("disable", () => {
    test("should set telemetry_enabled to false and write the config", (t) => {
      const mockWriteFileSync = t.mock.method(fs, "writeFileSync");
      telemetryConfig.disable();

      assert.equal(telemetryConfig.isEnabled(), false);
      assert.equal(mockWriteFileSync.mock.calls.length, 1);
      assert.deepStrictEqual(mockWriteFileSync.mock.calls[0].arguments, [
        "/path/to/config.json",
        JSON.stringify(telemetryConfig.config, null, 2),
      ]);
    });
  });

  describe("alertShown", () => {
    test("should return true if telemetry_alerted is defined", () => {
      telemetryConfig = new TelemetryConfig({
        configPath: "/path/to/config.json",
        config: {
          telemetry_enabled: true,
          telemetry_id: "123456",
          telemetry_salt: "private-salt",
          telemetry_alerted: new Date().toISOString(),
        },
      });

      const result = telemetryConfig.alertShown();

      assert.equal(result, true);
    });

    test("should set telemetry_alerted to current date and write the config if telemetry_alerted is undefined", (t) => {
      const mockWriteFileSync = mock.fn();
      t.mock.method(fs, "writeFileSync", mockWriteFileSync);
      const result = telemetryConfig.alertShown();

      assert.equal(result, true);
      assert.equal(telemetryConfig.hasSeenAlert(), true);
      assert.equal(mockWriteFileSync.mock.calls.length, 1);
      assert.deepEqual(mockWriteFileSync.mock.calls[0].arguments, [
        "/path/to/config.json",
        JSON.stringify(telemetryConfig.config, null, 2),
      ]);
    });
  });

  describe("oneWayHash", () => {
    test("should call oneWayHashWithSalt with the input and telemetry_salt from the config", (t) => {
      const mockOneWayHashWithSalt = mock.fn(() => "hashed-value");
      t.mock.method(utils, "oneWayHashWithSalt", mockOneWayHashWithSalt);

      const result = telemetryConfig.oneWayHash("input-value");
      assert.deepEqual(mockOneWayHashWithSalt.mock.calls[0].arguments, [
        {
          input: "input-value",
          salt: "private-salt",
        },
      ]);
      assert.equal(result, "hashed-value");
    });
  });

  describe("isDebug", () => {
    test("should return true if TURBO_TELEMETRY_DEBUG is set to '1'", () => {
      process.env.TURBO_TELEMETRY_DEBUG = "1";

      const result = TelemetryConfig.isDebug();

      assert.equal(result, true);
    });

    test("should return true if TURBO_TELEMETRY_DEBUG is set to 'true'", () => {
      process.env.TURBO_TELEMETRY_DEBUG = "true";

      const result = TelemetryConfig.isDebug();

      assert.equal(result, true);
    });

    test("should return false if TURBO_TELEMETRY_DEBUG is not set", () => {
      const result = TelemetryConfig.isDebug();

      assert.equal(result, false);
    });
  });
});
