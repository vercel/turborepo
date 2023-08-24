import { spyConsole, mockEnv } from "@turbo/test-utils";
import { getComparison } from "../src/getComparison";

describe("getComparison()", () => {
  mockEnv();
  const mockConsole = spyConsole();
  it("uses headRelative comparison when not running Vercel CI", () => {
    expect(getComparison({ workspace: "test-workspace" }))
      .toMatchInlineSnapshot(`
        Object {
          "ref": "HEAD^",
          "type": "headRelative",
        }
      `);
  });

  it("returns null when running in Vercel CI with no VERCEL_GIT_PREVIOUS_SHA", () => {
    process.env.VERCEL = "1";
    process.env.VERCEL_GIT_COMMIT_REF = "my-branch";
    expect(getComparison({ workspace: "test-workspace" })).toBeNull();
    expect(mockConsole.log).toHaveBeenCalledWith(
      "≫  ",
      'No previous deployments found for "test-workspace" on branch "my-branch"'
    );
  });

  it("uses custom fallback when running in Vercel CI with no VERCEL_GIT_PREVIOUS_SHA", () => {
    process.env.VERCEL = "1";
    process.env.VERCEL_GIT_COMMIT_REF = "my-branch";
    expect(getComparison({ workspace: "test-workspace", fallback: "HEAD^2" }))
      .toMatchInlineSnapshot(`
        Object {
          "ref": "HEAD^2",
          "type": "customFallback",
        }
      `);
    expect(mockConsole.log).toHaveBeenNthCalledWith(
      1,
      "≫  ",
      'No previous deployments found for "test-workspace" on branch "my-branch"'
    );
    expect(mockConsole.log).toHaveBeenNthCalledWith(
      2,
      "≫  ",
      "Falling back to ref HEAD^2"
    );
  });

  it("modifies output when running in Vercel CI with no VERCEL_GIT_PREVIOUS_SHA and no VERCEL_GIT_COMMIT_REF", () => {
    process.env.VERCEL = "1";
    expect(getComparison({ workspace: "test-workspace", fallback: "HEAD^2" }))
      .toMatchInlineSnapshot(`
        Object {
          "ref": "HEAD^2",
          "type": "customFallback",
        }
      `);
    expect(mockConsole.log).toHaveBeenNthCalledWith(
      1,
      "≫  ",
      'No previous deployments found for "test-workspace"'
    );
    expect(mockConsole.log).toHaveBeenNthCalledWith(
      2,
      "≫  ",
      "Falling back to ref HEAD^2"
    );
  });

  it("uses previousDeploy when running in Vercel CI with VERCEL_GIT_PREVIOUS_SHA", () => {
    process.env.VERCEL = "1";
    process.env.VERCEL_GIT_PREVIOUS_SHA = "mygitsha";
    process.env.VERCEL_GIT_COMMIT_REF = "my-branch";
    expect(getComparison({ workspace: "test-workspace" }))
      .toMatchInlineSnapshot(`
        Object {
          "ref": "mygitsha",
          "type": "previousDeploy",
        }
      `);
  });

  it("modifies output when running in Vercel CI with VERCEL_GIT_PREVIOUS_SHA but no VERCEL_GIT_COMMIT_REF", () => {
    process.env.VERCEL = "1";
    process.env.VERCEL_GIT_PREVIOUS_SHA = "mygitsha";
    expect(getComparison({ workspace: "test-workspace" }))
      .toMatchInlineSnapshot(`
        Object {
          "ref": "mygitsha",
          "type": "previousDeploy",
        }
      `);
    expect(mockConsole.log).toHaveBeenNthCalledWith(
      1,
      "≫  ",
      'Found previous deployment ("mygitsha") for "test-workspace"'
    );
  });
});
