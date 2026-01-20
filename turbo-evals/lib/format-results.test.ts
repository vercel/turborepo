import { describe, test, expect } from "bun:test";
import {
  formatClaudeCodeResultsTable,
  type EvalResult
} from "./format-results";

// Helper to calculate visual width of a string (counting emojis as 2 chars)
function visualWidth(str: string): number {
  let width = 0;
  for (const char of str) {
    // Check if character is an emoji
    // Common emoji ranges: https://en.wikipedia.org/wiki/Emoji#Unicode_blocks
    const code = char.codePointAt(0) || 0;
    const isEmoji =
      (code >= 0x1f300 && code <= 0x1f9ff) || // Misc Symbols and Pictographs, Emoticons, etc.
      (code >= 0x2600 && code <= 0x27bf) || // Misc symbols (✅❌ are here)
      (code >= 0x2300 && code <= 0x23ff) || // Misc Technical
      (code >= 0x2b50 && code <= 0x2b55); // Additional symbols

    if (isEmoji) {
      width += 2; // Emojis count as 2 visual chars
    } else {
      width += 1; // Regular chars count as 1
    }
  }
  return width;
}

describe("formatClaudeCodeResultsTable", () => {
  test("formats basic two-eval results with proper alignment", () => {
    const results: EvalResult[] = [
      {
        evalPath: "001-server-component",
        result: {
          buildSuccess: true,
          lintSuccess: true,
          testSuccess: true,
          duration: 17600
        }
      },
      {
        evalPath: "002-client-component",
        result: {
          buildSuccess: true,
          lintSuccess: true,
          testSuccess: false,
          duration: 14900
        }
      }
    ];

    const output = formatClaudeCodeResultsTable(results);
    const lines = output.split("\n");

    // Check that all lines have the same visual width (accounting for emoji double-width)
    const visualWidths = lines.map((line) => visualWidth(line));
    const allSame = visualWidths.every((w) => w === visualWidths[0]);
    expect(allSame).toBe(true);

    // Check header
    expect(lines[0]).toContain("Eval");
    expect(lines[0]).toContain("Claude Code");

    // Check separator
    expect(lines[1]).toMatch(/^\|[-]+\|[-]+\|$/);

    // Check data rows contain emojis and timing
    expect(lines[2]).toContain("001-server-component");
    expect(lines[2]).toContain("✅✅✅");
    expect(lines[2]).toContain("(17.6s)");

    expect(lines[3]).toContain("002-client-component");
    expect(lines[3]).toContain("✅✅❌");
    expect(lines[3]).toContain("(14.9s)");

    // Check separator before Overall
    expect(lines[4]).toMatch(/^\|[-]+\|[-]+\|$/);

    // Check Overall row
    expect(lines[5]).toContain("Overall (B/L/T)");
    expect(lines[5]).toContain("2/2/1");
    expect(lines[5]).toContain("100%");
    expect(lines[5]).toContain("50%");
  });

  test("handles varying timing lengths", () => {
    const results: EvalResult[] = [
      {
        evalPath: "fast-eval",
        result: {
          buildSuccess: true,
          lintSuccess: true,
          testSuccess: true,
          duration: 1000 // 1.0s
        }
      },
      {
        evalPath: "slow-eval",
        result: {
          buildSuccess: true,
          lintSuccess: true,
          testSuccess: true,
          duration: 123456 // 123.5s
        }
      }
    ];

    const output = formatClaudeCodeResultsTable(results);
    const lines = output.split("\n");

    // Check that all lines have the same visual width
    const visualWidths = lines.map((line) => visualWidth(line));
    const allSame = visualWidths.every((w) => w === visualWidths[0]);
    expect(allSame).toBe(true);

    expect(lines[2]).toContain("(1.0s)");
    expect(lines[3]).toContain("(123.5s)");
  });

  test("handles missing duration", () => {
    const results: EvalResult[] = [
      {
        evalPath: "no-duration-eval",
        result: {
          buildSuccess: true,
          lintSuccess: false,
          testSuccess: true
          // duration is undefined
        }
      }
    ];

    const output = formatClaudeCodeResultsTable(results);

    // Should show just emojis without duration when not available
    expect(output).toContain("✅❌✅");
    expect(output).not.toContain("(?s)");

    const lines = output.split("\n");
    const visualWidths = lines.map((line) => visualWidth(line));
    const allSame = visualWidths.every((w) => w === visualWidths[0]);
    expect(allSame).toBe(true);
  });

  test("handles long eval names", () => {
    const results: EvalResult[] = [
      {
        evalPath: "very-long-eval-name-that-exceeds-default-width",
        result: {
          buildSuccess: true,
          lintSuccess: true,
          testSuccess: true,
          duration: 5000
        }
      }
    ];

    const output = formatClaudeCodeResultsTable(results);
    const lines = output.split("\n");

    // Check that all lines have the same visual width
    const visualWidths = lines.map((line) => visualWidth(line));
    const allSame = visualWidths.every((w) => w === visualWidths[0]);
    expect(allSame).toBe(true);

    // Check the long name is present and not truncated
    expect(lines[2]).toContain(
      "very-long-eval-name-that-exceeds-default-width"
    );
  });

  test("calculates percentages correctly", () => {
    const results: EvalResult[] = [
      {
        evalPath: "eval1",
        result: {
          buildSuccess: true,
          lintSuccess: true,
          testSuccess: true,
          duration: 1000
        }
      },
      {
        evalPath: "eval2",
        result: {
          buildSuccess: true,
          lintSuccess: false,
          testSuccess: true,
          duration: 1000
        }
      },
      {
        evalPath: "eval3",
        result: {
          buildSuccess: false,
          lintSuccess: true,
          testSuccess: false,
          duration: 1000
        }
      }
    ];

    const output = formatClaudeCodeResultsTable(results);

    // 3/3 builds = 100% (but one failed so 2/3 = 67%)
    // 2/3 lints = 67%
    // 2/3 tests = 67%
    expect(output).toContain("Overall (B/L/T)");
    expect(output).toContain("2/2/2"); // 2 pass each category
    expect(output).toContain("67%"); // Should have at least one 67%
  });

  test("handles 5-character duration strings like (21.9s)", () => {
    const results: EvalResult[] = [
      {
        evalPath: "001-server-component",
        result: {
          buildSuccess: true,
          lintSuccess: true,
          testSuccess: true,
          duration: 21900 // 21.9s
        }
      },
      {
        evalPath: "002-client-component",
        result: {
          buildSuccess: true,
          lintSuccess: true,
          testSuccess: true,
          duration: 30700 // 30.7s
        }
      }
    ];

    const output = formatClaudeCodeResultsTable(results);
    const lines = output.split("\n");

    // Check that all lines have the same visual width (accounting for emoji double-width)
    const visualWidths = lines.map((line) => visualWidth(line));
    const allSame = visualWidths.every((w) => w === visualWidths[0]);
    expect(allSame).toBe(true);

    // Check the timing strings are present
    expect(lines[2]).toContain("(21.9s)");
    expect(lines[3]).toContain("(30.7s)");
  });
});
