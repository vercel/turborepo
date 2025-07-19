import {
  describe,
  test,
  expect,
  beforeEach,
  afterEach,
  jest,
} from "@jest/globals";
import {
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
      // Should contain ANSI 256-color escape sequence and reset
      expect(result).toContain("\x1b[38;5;");
      expect(result).toContain("\x1b[0m");
    });

    test("turboRed should apply red color formatting", () => {
      const result = turboRed("test");
      expect(result).toContain("test");
      // Should contain ANSI 256-color escape sequence and reset
      expect(result).toContain("\x1b[38;5;");
      expect(result).toContain("\x1b[0m");
    });

    test("yellow should apply yellow color formatting", () => {
      const result = yellow("test");
      expect(result).toContain("test");
      // Should contain ANSI 256-color escape sequence and reset
      expect(result).toContain("\x1b[38;5;");
      expect(result).toContain("\x1b[0m");
    });
  });

  describe("logging functions", () => {
    test("log should call console.log with arguments", () => {
      log("test", "message");
      expect(mockConsoleLog).toHaveBeenCalledWith("test", "message");
    });

    test("info should log with blue bold >>> prefix", () => {
      info("test message");
      expect(mockConsoleLog).toHaveBeenCalledTimes(1);
      const [prefix, message] = mockConsoleLog.mock.calls[0];
      // Should contain blue color codes and bold formatting
      expect(prefix).toContain("\x1b[38;5;");
      expect(prefix).toContain("\x1b[1m");
      expect(prefix).toContain(">>>");
      expect(message).toBe("test message");
    });

    test("warn should call console.error with yellow bold >>> prefix", () => {
      warn("warning message");
      expect(mockConsoleError).toHaveBeenCalledTimes(1);
      const [prefix, message] = mockConsoleError.mock.calls[0];
      // Should contain yellow color codes and bold formatting
      expect(prefix).toContain("\x1b[38;5;");
      expect(prefix).toContain("\x1b[1m");
      expect(prefix).toContain(">>>");
      expect(message).toBe("warning message");
    });

    test("error should call console.error with red bold >>> prefix", () => {
      error("error message");
      expect(mockConsoleError).toHaveBeenCalledTimes(1);
      const [prefix, message] = mockConsoleError.mock.calls[0];
      // Should contain red color codes and bold formatting
      expect(prefix).toContain("\x1b[38;5;");
      expect(prefix).toContain("\x1b[1m");
      expect(prefix).toContain(">>>");
      expect(message).toBe("error message");
    });

    test("item should log with blue bold bullet point prefix", () => {
      item("item message");
      expect(mockConsoleLog).toHaveBeenCalledTimes(1);
      const [prefix, message] = mockConsoleLog.mock.calls[0];
      // Should contain blue color codes and bold formatting
      expect(prefix).toContain("\x1b[38;5;");
      expect(prefix).toContain("\x1b[1m");
      expect(prefix).toContain("•");
      expect(message).toBe("item message");
    });

    test("info should handle multiple arguments by joining them", () => {
      info("arg1", "arg2", "arg3");
      expect(mockConsoleLog).toHaveBeenCalledTimes(1);
      const [_prefix, message] = mockConsoleLog.mock.calls[0];
      expect(message).toBe("arg1 arg2 arg3");
    });

    test("warn should handle multiple arguments by joining them", () => {
      warn("warning", "with", "details");
      expect(mockConsoleError).toHaveBeenCalledTimes(1);
      const [_prefix, message] = mockConsoleError.mock.calls[0];
      expect(message).toBe("warning with details");
    });

    test("error should handle multiple arguments by joining them", () => {
      error("error", "with", "context");
      expect(mockConsoleError).toHaveBeenCalledTimes(1);
      const [_prefix, message] = mockConsoleError.mock.calls[0];
      expect(message).toBe("error with context");
    });

    test("item should handle multiple arguments by joining them", () => {
      item("item", "with", "details");
      expect(mockConsoleLog).toHaveBeenCalledTimes(1);
      const [_prefix, message] = mockConsoleLog.mock.calls[0];
      expect(message).toBe("item with details");
    });
  });

  describe("text formatting functions", () => {
    test("bold should log text with bold ANSI formatting", () => {
      bold("bold text");
      expect(mockConsoleLog).toHaveBeenCalledTimes(1);
      const [formattedText] = mockConsoleLog.mock.calls[0];
      expect(formattedText).toContain("bold text");
      expect(formattedText).toContain("\x1b[1m"); // Bold start ANSI code
      expect(formattedText).toContain("\x1b[22m"); // Bold end ANSI code
    });

    test("underline should log text with underline ANSI formatting", () => {
      underline("underlined text");
      expect(mockConsoleLog).toHaveBeenCalledTimes(1);
      const [formattedText] = mockConsoleLog.mock.calls[0];
      expect(formattedText).toContain("underlined text");
      expect(formattedText).toContain("\x1b[4m"); // Underline start ANSI code
      expect(formattedText).toContain("\x1b[24m"); // Underline end ANSI code
    });

    test("dimmed should log text with dim ANSI formatting", () => {
      dimmed("dimmed text");
      expect(mockConsoleLog).toHaveBeenCalledTimes(1);
      const [formattedText] = mockConsoleLog.mock.calls[0];
      expect(formattedText).toContain("dimmed text");
      expect(formattedText).toContain("\x1b[2m"); // Dim start ANSI code
      expect(formattedText).toContain("\x1b[22m"); // Dim end ANSI code
    });

    test("grey should log text with grey color ANSI formatting", () => {
      grey("grey text");
      expect(mockConsoleLog).toHaveBeenCalledTimes(1);
      const [formattedText] = mockConsoleLog.mock.calls[0];
      expect(formattedText).toContain("grey text");
      expect(formattedText).toContain("\x1b[90m"); // Grey start ANSI code
      expect(formattedText).toContain("\x1b[39m"); // Grey end ANSI code
    });

    test("bold should handle multiple arguments by joining them", () => {
      bold("bold", "text", "here");
      expect(mockConsoleLog).toHaveBeenCalledTimes(1);
      const [formattedText] = mockConsoleLog.mock.calls[0];
      expect(formattedText).toContain("bold text here");
    });

    test("underline should handle multiple arguments by joining them", () => {
      underline("underlined", "text", "here");
      expect(mockConsoleLog).toHaveBeenCalledTimes(1);
      const [formattedText] = mockConsoleLog.mock.calls[0];
      expect(formattedText).toContain("underlined text here");
    });

    test("dimmed should handle multiple arguments by joining them", () => {
      dimmed("dimmed", "text", "here");
      expect(mockConsoleLog).toHaveBeenCalledTimes(1);
      const [formattedText] = mockConsoleLog.mock.calls[0];
      expect(formattedText).toContain("dimmed text here");
    });

    test("grey should handle multiple arguments by joining them", () => {
      grey("grey", "text", "here");
      expect(mockConsoleLog).toHaveBeenCalledTimes(1);
      const [formattedText] = mockConsoleLog.mock.calls[0];
      expect(formattedText).toContain("grey text here");
    });
  });
});
