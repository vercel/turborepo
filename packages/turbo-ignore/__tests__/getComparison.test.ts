import { getComparison } from "../src/getComparison";
import { spyConsole, validateLogs } from "./test-utils";

describe("getComparison()", () => {
  const mockConsole = spyConsole();
  it("uses headRelative comparison when not running Vercel CI", async () => {
    expect(getComparison({ workspace: "test-workspace" }))
      .toMatchInlineSnapshot(`
        Object {
          "ref": "HEAD^",
          "type": "headRelative",
        }
      `);
  });

  it("returns null when running in Vercel CI with no VERCEL_GIT_PREVIOUS_SHA and fallback disabled", async () => {
    process.env.VERCEL = "1";
    process.env.VERCEL_GIT_COMMIT_REF = "my-branch";
    expect(
      getComparison({ workspace: "test-workspace", fallback: "false" })
    ).toBeNull();
    expect(mockConsole.log).toHaveBeenCalledWith(
      "≫  ",
      'no previous deployments found for "test-workspace" on "my-branch".'
    );
  });

  it("uses default fallback when running in Vercel CI with no VERCEL_GIT_PREVIOUS_SHA", async () => {
    process.env.VERCEL = "1";
    process.env.VERCEL_GIT_COMMIT_REF = "my-branch";
    expect(getComparison({ workspace: "test-workspace" }))
      .toMatchInlineSnapshot(`
        Object {
          "ref": "HEAD^",
          "type": "customFallback",
        }
      `);

    validateLogs(
      [
        'no previous deployments found for "test-workspace" on "my-branch".',
        "falling back to HEAD^",
      ],
      mockConsole.log
    );
  });

  it("uses custom fallback when running in Vercel CI with no VERCEL_GIT_PREVIOUS_SHA", async () => {
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
      'no previous deployments found for "test-workspace" on "my-branch".'
    );
    expect(mockConsole.log).toHaveBeenNthCalledWith(
      2,
      "≫  ",
      "falling back to HEAD^2"
    );
  });

  it("uses previousDeploy when running in Vercel CI with VERCEL_GIT_PREVIOUS_SHA", async () => {
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
});
