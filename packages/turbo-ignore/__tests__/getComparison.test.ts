// eslint-disable-next-line camelcase -- This is a test file
import child_process from "node:child_process";
import { spyConsole, mockEnv, validateLogs } from "@turbo/test-utils";
import { describe, it, expect, jest } from "@jest/globals";
import { getComparison } from "../src/getComparison";

describe("getComparison()", () => {
  mockEnv();
  const mockConsole = spyConsole();
  it("uses headRelative comparison when not running Vercel CI", () => {
    expect(getComparison({ workspace: "test-workspace" }))
      .toMatchInlineSnapshot(`
      {
        "ref": "HEAD^",
        "type": "headRelative",
      }
    `);
    expect(mockConsole.log).toHaveBeenCalledTimes(0);
  });

  it("uses fallback comparison if provided when not running Vercel CI", () => {
    expect(getComparison({ workspace: "test-workspace", fallback: "HEAD^2" }))
      .toMatchInlineSnapshot(`
      {
        "ref": "HEAD^2",
        "type": "customFallback",
      }
    `);
    expect(mockConsole.log).toHaveBeenNthCalledWith(
      1,
      "≫  ",
      "Falling back to ref HEAD^2"
    );
  });

  it("returns null when running in Vercel CI with no VERCEL_GIT_PREVIOUS_SHA", () => {
    process.env.VERCEL = "1";
    process.env.VERCEL_GIT_COMMIT_REF = "my-branch";
    expect(getComparison({ workspace: "test-workspace" })).toBeNull();

    expect(mockConsole.log).toHaveBeenNthCalledWith(
      1,
      "≫  ",
      'No previous deployments found for "test-workspace" on branch "my-branch"'
    );
  });

  it("uses custom fallback when running in Vercel CI with no VERCEL_GIT_PREVIOUS_SHA", () => {
    process.env.VERCEL = "1";
    process.env.VERCEL_GIT_COMMIT_REF = "my-branch";
    expect(getComparison({ workspace: "test-workspace", fallback: "HEAD^2" }))
      .toMatchInlineSnapshot(`
      {
        "ref": "HEAD^2",
        "type": "customFallback",
      }
    `);

    validateLogs(mockConsole.log, [
      [
        "≫  ",
        'No previous deployments found for "test-workspace" on branch "my-branch"',
      ],
      ["≫  ", "Falling back to ref HEAD^2"],
    ]);
  });

  it("modifies output when running in Vercel CI with no VERCEL_GIT_PREVIOUS_SHA and no VERCEL_GIT_COMMIT_REF", () => {
    process.env.VERCEL = "1";
    expect(getComparison({ workspace: "test-workspace", fallback: "HEAD^2" }))
      .toMatchInlineSnapshot(`
      {
        "ref": "HEAD^2",
        "type": "customFallback",
      }
    `);

    validateLogs(mockConsole.log, [
      ["≫  ", 'No previous deployments found for "test-workspace"'],
      ["≫  ", "Falling back to ref HEAD^2"],
    ]);
  });

  it("uses previousDeploy when running in Vercel CI with VERCEL_GIT_PREVIOUS_SHA", () => {
    const mockExec = jest
      .spyOn(child_process, "execSync")
      .mockReturnValue("commit");

    process.env.VERCEL = "1";
    process.env.VERCEL_GIT_PREVIOUS_SHA = "mygitsha";
    process.env.VERCEL_GIT_COMMIT_REF = "my-branch";
    expect(getComparison({ workspace: "test-workspace" }))
      .toMatchInlineSnapshot(`
      {
        "ref": "mygitsha",
        "type": "previousDeploy",
      }
    `);

    expect(mockConsole.log).toHaveBeenNthCalledWith(
      1,
      "≫  ",
      'Found previous deployment ("mygitsha") for "test-workspace" on branch "my-branch"'
    );

    mockExec.mockRestore();
  });

  it("uses fallback when running in Vercel CI with unreachable VERCEL_GIT_PREVIOUS_SHA", () => {
    const mockExec = jest
      .spyOn(child_process, "execSync")
      .mockImplementation(() => {
        throw new Error("fatal: Not a valid object name mygitsha");
      });

    process.env.VERCEL = "1";
    process.env.VERCEL_GIT_PREVIOUS_SHA = "mygitsha";
    process.env.VERCEL_GIT_COMMIT_REF = "my-branch";
    expect(getComparison({ workspace: "test-workspace", fallback: "HEAD^2" }))
      .toMatchInlineSnapshot(`
      {
        "ref": "HEAD^2",
        "type": "customFallback",
      }
    `);

    validateLogs(mockConsole.log, [
      [
        "≫  ",
        'Previous deployment ("mygitsha") for "test-workspace" on branch "my-branch" is unreachable.',
      ],
      ["≫  ", "Falling back to ref HEAD^2"],
    ]);

    mockExec.mockRestore();
  });

  it("returns null running in Vercel CI with unreachable VERCEL_GIT_PREVIOUS_SHA and no fallback", () => {
    const mockExec = jest
      .spyOn(child_process, "execSync")
      .mockImplementation(() => {
        throw new Error("fatal: Not a valid object name mygitsha");
      });

    process.env.VERCEL = "1";
    process.env.VERCEL_GIT_PREVIOUS_SHA = "mygitsha";
    process.env.VERCEL_GIT_COMMIT_REF = "my-branch";
    expect(getComparison({ workspace: "test-workspace" })).toBeNull();

    expect(mockConsole.log).toHaveBeenNthCalledWith(
      1,
      "≫  ",
      'Previous deployment ("mygitsha") for "test-workspace" on branch "my-branch" is unreachable.'
    );

    mockExec.mockRestore();
  });

  it("modifies output when running in Vercel CI with VERCEL_GIT_PREVIOUS_SHA but no VERCEL_GIT_COMMIT_REF", () => {
    const mockExec = jest
      .spyOn(child_process, "execSync")
      .mockReturnValue("commit");

    process.env.VERCEL = "1";
    process.env.VERCEL_GIT_PREVIOUS_SHA = "mygitsha";
    expect(getComparison({ workspace: "test-workspace" }))
      .toMatchInlineSnapshot(`
      {
        "ref": "mygitsha",
        "type": "previousDeploy",
      }
    `);

    expect(mockConsole.log).toHaveBeenNthCalledWith(
      1,
      "≫  ",
      'Found previous deployment ("mygitsha") for "test-workspace"'
    );

    mockExec.mockRestore();
  });

  it("modifies output when running in Vercel CI with unreachable VERCEL_GIT_PREVIOUS_SHA and no VERCEL_GIT_COMMIT_REF", () => {
    const mockExec = jest
      .spyOn(child_process, "execSync")
      .mockImplementation(() => {
        throw new Error("fatal: Not a valid object name mygitsha");
      });

    process.env.VERCEL = "1";
    process.env.VERCEL_GIT_PREVIOUS_SHA = "mygitsha";
    expect(getComparison({ workspace: "test-workspace" })).toBeNull();

    expect(mockConsole.log).toHaveBeenNthCalledWith(
      1,
      "≫  ",
      'Previous deployment ("mygitsha") for "test-workspace" is unreachable.'
    );

    mockExec.mockRestore();
  });
});
