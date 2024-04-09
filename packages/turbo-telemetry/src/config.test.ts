import * as fs from "node:fs";
import { TelemetryConfig } from "./config";
import * as utils from "./utils";

jest.mock("node:fs");

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
    jest.resetAllMocks();

    delete process.env.DO_NOT_TRACK;
    delete process.env.TURBO_TELEMETRY_DISABLED;
    delete process.env.TURBO_TELEMETRY_MESSAGE_DISABLED;
    delete process.env.TURBO_TELEMETRY_DEBUG;
  });

  describe("fromDefaultConfig", () => {
    test("should create TelemetryConfig instance from default config", async () => {
      const mockConfigPath = "/path/to/defaultConfig.json";
      const mockFileContent = JSON.stringify({
        telemetry_enabled: true,
        telemetry_id: "654321",
        telemetry_salt: "default-salt",
      });

      const mockDefaultConfigPath = jest.fn().mockResolvedValue(mockConfigPath);
      const mockReadFileSync = jest.fn().mockReturnValue(mockFileContent);

      jest
        .spyOn(utils, "defaultConfigPath")
        .mockImplementation(mockDefaultConfigPath);
      jest.spyOn(fs, "readFileSync").mockImplementation(mockReadFileSync);

      const result = await TelemetryConfig.fromDefaultConfig();

      expect(mockDefaultConfigPath).toHaveBeenCalled();
      expect(mockReadFileSync).toHaveBeenCalledWith(mockConfigPath, "utf-8");
      expect(result).toBeInstanceOf(TelemetryConfig);
      expect(result?.id).toEqual("654321");
    });

    test("should generate new config if default config doesn't exist", async () => {
      const mockConfigPath = "/path/to/defaultConfig.json";
      const mockDefaultConfigPath = jest.fn().mockResolvedValue(mockConfigPath);
      const mockReadFileSync = jest.fn().mockImplementation(() => {
        throw new Error("File not found");
      });
      const mockRmSync = jest.fn();
      const mockWriteFileSync = jest.fn();

      jest
        .spyOn(utils, "defaultConfigPath")
        .mockImplementation(mockDefaultConfigPath);
      jest.spyOn(fs, "readFileSync").mockImplementation(mockReadFileSync);
      jest.spyOn(fs, "rmSync").mockImplementation(mockRmSync);
      jest.spyOn(fs, "writeFileSync").mockImplementation(mockWriteFileSync);

      const result = await TelemetryConfig.fromDefaultConfig();

      expect(mockDefaultConfigPath).toHaveBeenCalled();
      expect(mockReadFileSync).toHaveBeenCalledWith(mockConfigPath, "utf-8");
      expect(mockRmSync).toHaveBeenCalled();
      expect(mockRmSync).toHaveBeenCalledWith(mockConfigPath, {
        force: true,
      });
      expect(mockWriteFileSync).toHaveBeenCalled();
      expect(mockWriteFileSync).toHaveBeenCalledWith(
        mockConfigPath,
        expect.any(String)
      );
      expect(result).toBeInstanceOf(TelemetryConfig);
      expect(result?.id).toEqual(expect.any(String));
      expect(result?.config.telemetry_enabled).toEqual(true);
    });

    test("should not throw if default config is missing a key", async () => {
      const mockConfigPath = "/path/to/defaultConfig.json";
      const id = "654321";
      const mockFileContent = JSON.stringify({
        // missing telemetry_enabled
        telemetry_id: id,
        telemetry_salt: "default-salt",
      });
      const mockRmSync = jest.fn();
      const mockWriteFileSync = jest.fn();

      const mockDefaultConfigPath = jest.fn().mockResolvedValue(mockConfigPath);
      const mockReadFileSync = jest.fn().mockReturnValue(mockFileContent);

      jest
        .spyOn(utils, "defaultConfigPath")
        .mockImplementation(mockDefaultConfigPath);
      jest.spyOn(fs, "readFileSync").mockImplementation(mockReadFileSync);
      jest.spyOn(fs, "rmSync").mockImplementation(mockRmSync);
      jest.spyOn(fs, "writeFileSync").mockImplementation(mockWriteFileSync);

      const result = await TelemetryConfig.fromDefaultConfig();

      expect(mockDefaultConfigPath).toHaveBeenCalled();
      expect(mockReadFileSync).toHaveBeenCalledWith(mockConfigPath, "utf-8");
      expect(mockRmSync).toHaveBeenCalled();
      expect(mockRmSync).toHaveBeenCalledWith(mockConfigPath, {
        force: true,
      });
      expect(mockWriteFileSync).toHaveBeenCalled();
      expect(mockWriteFileSync).toHaveBeenCalledWith(
        mockConfigPath,
        expect.any(String)
      );
      expect(result).toBeInstanceOf(TelemetryConfig);
      expect(result?.id).toEqual(expect.any(String));
      // this shouldn't match because we threw away the file and made a new one
      expect(result?.id).not.toEqual(id);
      expect(result?.config.telemetry_enabled).toEqual(true);
    });

    test("should not throw if default config has a key of the wrong type", async () => {
      const mockConfigPath = "/path/to/defaultConfig.json";
      const salt = "default-salt";
      const mockFileContent = JSON.stringify({
        telemetry_enabled: true,
        // telemetry_id should be a string
        telemetry_id: true,
        telemetry_salt: salt,
      });
      const mockRmSync = jest.fn();
      const mockWriteFileSync = jest.fn();

      const mockDefaultConfigPath = jest.fn().mockResolvedValue(mockConfigPath);
      const mockReadFileSync = jest.fn().mockReturnValue(mockFileContent);

      jest
        .spyOn(utils, "defaultConfigPath")
        .mockImplementation(mockDefaultConfigPath);
      jest.spyOn(fs, "readFileSync").mockImplementation(mockReadFileSync);
      jest.spyOn(fs, "rmSync").mockImplementation(mockRmSync);
      jest.spyOn(fs, "writeFileSync").mockImplementation(mockWriteFileSync);

      const result = await TelemetryConfig.fromDefaultConfig();

      expect(mockDefaultConfigPath).toHaveBeenCalled();
      expect(mockReadFileSync).toHaveBeenCalledWith(mockConfigPath, "utf-8");
      expect(mockRmSync).toHaveBeenCalled();
      expect(mockRmSync).toHaveBeenCalledWith(mockConfigPath, {
        force: true,
      });
      expect(mockWriteFileSync).toHaveBeenCalled();
      expect(mockWriteFileSync).toHaveBeenCalledWith(
        mockConfigPath,
        expect.any(String)
      );
      expect(result).toBeInstanceOf(TelemetryConfig);
      expect(result?.id).toEqual(expect.any(String));
      // this shouldn't match because we threw away the file and made a new one
      expect(result?.config.telemetry_salt).not.toEqual(salt);
      expect(result?.config.telemetry_enabled).toEqual(true);
    });
  });

  describe("write", () => {
    test("should write the config to the file", () => {
      const mockWriteFileSync = jest.fn();
      jest.spyOn(fs, "writeFileSync").mockImplementation(mockWriteFileSync);

      const mockJson = JSON.stringify(telemetryConfig.config, null, 2);
      telemetryConfig.tryWrite();

      expect(mockWriteFileSync).toHaveBeenCalledWith(
        "/path/to/config.json",
        mockJson
      );
    });

    test("should not throw if write fails", () => {
      const mockWriteFileSync = jest.fn();
      jest.spyOn(fs, "writeFileSync").mockImplementation(mockWriteFileSync);
      mockWriteFileSync.mockImplementation(() => {
        throw new Error("Write error");
      });

      const mockJson = JSON.stringify(telemetryConfig.config, null, 2);
      // this shouldn't throw
      telemetryConfig.tryWrite();

      expect(mockWriteFileSync).toHaveBeenCalledWith(
        "/path/to/config.json",
        mockJson
      );
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
          telemetry_alerted: new Date(),
        },
      });

      const result = telemetryConfig.hasSeenAlert();

      expect(result).toBe(true);
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

      expect(result).toBe(false);
    });

    test("should return false if telemetry_alerted is undefined", () => {
      const result = telemetryConfig.hasSeenAlert();

      expect(result).toBe(false);
    });
  });

  describe("isEnabled", () => {
    test.each([
      { envVar: "DO_NOT_TRACK", value: "1", expectedResult: false },
      { envVar: "DO_NOT_TRACK", value: "true", expectedResult: false },
      { envVar: "TURBO_TELEMETRY_DISABLED", value: "1", expectedResult: false },
      {
        envVar: "TURBO_TELEMETRY_DISABLED",
        value: "true",
        expectedResult: false,
      },
      { envVar: null, value: null, expectedResult: true },
    ])(
      "should return $expectedResult when $envVar is set to '$value'",
      ({ envVar, value, expectedResult }) => {
        if (envVar) {
          process.env[envVar] = value;
        }

        const result = telemetryConfig.isEnabled();
        expect(result).toBe(expectedResult);
      }
    );
  });

  describe("isTelemetryWarningEnabled", () => {
    test("should return false if TURBO_TELEMETRY_MESSAGE_DISABLED is set to '1'", () => {
      process.env.TURBO_TELEMETRY_MESSAGE_DISABLED = "1";

      const result = telemetryConfig.isTelemetryWarningEnabled();

      expect(result).toBe(false);
    });

    test("should return false if TURBO_TELEMETRY_MESSAGE_DISABLED is set to 'true'", () => {
      process.env.TURBO_TELEMETRY_MESSAGE_DISABLED = "true";

      const result = telemetryConfig.isTelemetryWarningEnabled();

      expect(result).toBe(false);
    });

    test("should return true if TURBO_TELEMETRY_MESSAGE_DISABLED is not set", () => {
      const result = telemetryConfig.isTelemetryWarningEnabled();

      expect(result).toBe(true);
    });
  });

  describe("showAlert", () => {
    test("should log the telemetry alert if conditions are met", () => {
      const mockLog = jest.spyOn(console, "log").mockImplementation();
      telemetryConfig.showAlert();
      expect(mockLog).toHaveBeenCalledTimes(6);
    });

    test("should not log the telemetry alert if conditions are not met", () => {
      const mockLog = jest.spyOn(console, "log").mockImplementation();

      telemetryConfig = new TelemetryConfig({
        configPath: "/path/to/config.json",
        config: {
          telemetry_enabled: false,
          telemetry_id: "123456",
          telemetry_salt: "private-salt",
        },
      });

      telemetryConfig.showAlert();

      expect(mockLog).not.toHaveBeenCalled();
      expect(mockLog).not.toHaveBeenCalled();
      expect(mockLog).not.toHaveBeenCalled();
      expect(mockLog).not.toHaveBeenCalled();
    });
  });

  describe("enable", () => {
    test("should set telemetry_enabled to true and write the config", () => {
      const mockWriteFileSync = jest.fn();
      jest.spyOn(fs, "writeFileSync").mockImplementation(mockWriteFileSync);

      telemetryConfig.enable();
      expect(telemetryConfig.isEnabled()).toBe(true);
      expect(mockWriteFileSync).toHaveBeenCalled();
      expect(mockWriteFileSync).toHaveBeenCalledWith(
        "/path/to/config.json",
        JSON.stringify(telemetryConfig.config, null, 2)
      );
    });
  });

  describe("disable", () => {
    test("should set telemetry_enabled to false and write the config", () => {
      const mockWriteFileSync = jest.fn();
      jest.spyOn(fs, "writeFileSync").mockImplementation(mockWriteFileSync);
      telemetryConfig.disable();

      expect(telemetryConfig.isEnabled()).toBe(false);
      expect(mockWriteFileSync).toHaveBeenCalled();
      expect(mockWriteFileSync).toHaveBeenCalledWith(
        "/path/to/config.json",
        JSON.stringify(telemetryConfig.config, null, 2)
      );
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
          telemetry_alerted: new Date(),
        },
      });

      const result = telemetryConfig.alertShown();

      expect(result).toBe(true);
    });

    test("should set telemetry_alerted to current date and write the config if telemetry_alerted is undefined", () => {
      const mockWriteFileSync = jest.fn();
      const mockDate = jest.fn().mockReturnValue("2021-01-01T00:00:00.000Z");
      jest.spyOn(fs, "writeFileSync").mockImplementation(mockWriteFileSync);
      jest.spyOn(global, "Date").mockImplementation(mockDate);

      const result = telemetryConfig.alertShown();

      expect(result).toBe(true);
      expect(telemetryConfig.hasSeenAlert()).toBe(true);
      expect(mockWriteFileSync).toHaveBeenCalled();
      expect(mockWriteFileSync).toHaveBeenCalledWith(
        "/path/to/config.json",
        JSON.stringify(telemetryConfig.config, null, 2)
      );
    });
  });

  describe("oneWayHash", () => {
    test("should call oneWayHashWithSalt with the input and telemetry_salt from the config", () => {
      const mockOneWayHashWithSalt = jest.fn().mockReturnValue("hashed-value");
      jest
        .spyOn(utils, "oneWayHashWithSalt")
        .mockImplementation(mockOneWayHashWithSalt);

      const result = telemetryConfig.oneWayHash("input-value");
      expect(mockOneWayHashWithSalt).toHaveBeenCalledWith({
        input: "input-value",
        salt: "private-salt",
      });
      expect(result).toBe("hashed-value");
    });
  });

  describe("isDebug", () => {
    test("should return true if TURBO_TELEMETRY_DEBUG is set to '1'", () => {
      process.env.TURBO_TELEMETRY_DEBUG = "1";

      const result = TelemetryConfig.isDebug();

      expect(result).toBe(true);
    });

    test("should return true if TURBO_TELEMETRY_DEBUG is set to 'true'", () => {
      process.env.TURBO_TELEMETRY_DEBUG = "true";

      const result = TelemetryConfig.isDebug();

      expect(result).toBe(true);
    });

    test("should return false if TURBO_TELEMETRY_DEBUG is not set", () => {
      const result = TelemetryConfig.isDebug();

      expect(result).toBe(false);
    });
  });
});
