import {
  describe,
  test,
  expect,
  beforeEach,
  afterEach,
  jest,
} from "@jest/globals";
import {
  turboGradient,
  turboBlue,
  turboRed,
  yellow,
  turboLoader,
  info,
  bold,
  underline,
  dimmed,
  grey,
  item,
  log,
  warn,
  error,
} from "../src/logger";

// Mock console methods
const mockConsoleLog = jest.fn();
const mockConsoleError = jest.fn();

beforeEach(() => {
  jest.clearAllMocks();
  jest.spyOn(console, "log").mockImplementation(mockConsoleLog);
  jest.spyOn(console, "error").mockImplementation(mockConsoleError);
});

afterEach(() => {
  jest.restoreAllMocks();
});

describe("logger utilities", () => {
  describe("color functions", () => {
    test("turboBlue should apply blue color formatting", () => {
      const result = turboBlue("test");
      expect(result).toContain("test");
      expect(result).toMatch(/\x1b\[38;5;\d+m.*\x1b\[0m/);
    });

    test("turboRed should apply red color formatting", () => {
      const result = turboRed("test");
      expect(result).toContain("test");
      expect(result).toMatch(/\x1b\[38;5;\d+m.*\x1b\[0m/);
    });

    test("yellow should apply yellow color formatting", () => {
      const result = yellow("test");
      expect(result).toContain("test");
      expect(result).toMatch(/\x1b\[38;5;\d+m.*\x1b\[0m/);
    });

    test("turboGradient should create gradient text", () => {
      const result = turboGradient("test");
      expect(typeof result).toBe("string");
      expect(result.length).toBeGreaterThan(0);
    });
  });

  describe("logging functions", () => {
    test("log should call console.log with arguments", () => {
      log("test", "message");
      expect(mockConsoleLog).toHaveBeenCalledWith("test", "message");
    });

    test("info should log with turbo blue prefix", () => {
      info("test message");
      expect(mockConsoleLog).toHaveBeenCalledWith(
        expect.stringContaining(">>>"),
        "test message"
      );
    });

    test("warn should call console.error with yellow formatting", () => {
      warn("warning message");
      expect(mockConsoleError).toHaveBeenCalledWith(
        expect.stringContaining(">>>"),
        "warning message"
      );
    });

    test("error should call console.error with red formatting", () => {
      error("error message");
      expect(mockConsoleError).toHaveBeenCalledWith(
        expect.stringContaining(">>>"),
        "error message"
      );
    });

    test("item should log with bullet point prefix", () => {
      item("item message");
      expect(mockConsoleLog).toHaveBeenCalledWith(
        expect.stringContaining("•"),
        "item message"
      );
    });
  });

  describe("text formatting functions", () => {
    test("bold should log bold formatted text", () => {
      bold("bold text");
      expect(mockConsoleLog).toHaveBeenCalledWith(expect.any(String));
    });

    test("underline should log underlined text", () => {
      underline("underlined text");
      expect(mockConsoleLog).toHaveBeenCalledWith(expect.any(String));
    });

    test("dimmed should log dimmed text", () => {
      dimmed("dimmed text");
      expect(mockConsoleLog).toHaveBeenCalledWith(expect.any(String));
    });

    test("grey should log grey colored text", () => {
      grey("grey text");
      expect(mockConsoleLog).toHaveBeenCalledWith(expect.any(String));
    });
  });

  describe("turboLoader", () => {
    test("should create ora spinner with custom frames", () => {
      const loader = turboLoader("Loading...");
      expect(loader).toBeDefined();
      expect(loader.text).toBe("Loading...");
      expect(loader.spinner).toBeDefined();
      expect(typeof loader.spinner).toBe("object");
    });
  });
});
