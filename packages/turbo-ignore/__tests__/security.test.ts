// eslint-disable-next-line camelcase -- This is a test file
import child_process from "node:child_process";
import { existsSync, unlinkSync } from "node:fs";
import {
  describe,
  it,
  expect,
  jest,
  beforeEach,
  afterEach,
} from "@jest/globals";
import { validateSHAExists } from "../src/getComparison";
import { mockEnv } from "@turbo/test-utils";

describe("Security: Command Injection Prevention", () => {
  mockEnv();

  describe("validateSHAExists()", () => {
    it("uses real execFileSync to validate HEAD ref", () => {
      const result = validateSHAExists("HEAD");
      expect(result).toBe(true); // HEAD should exist in any git repository
    });

    it("returns false for invalid ref using real execFileSync", () => {
      const result = validateSHAExists(
        "this-ref-definitely-does-not-exist-12345"
      );
      expect(result).toBe(false);
    });

    it("safely handles malicious ref with command injection attempt", () => {
      const execFileSyncSpy = jest.spyOn(child_process, "execFileSync");
      const maliciousRef = "HEAD]; touch /tmp/pwned; echo [";
      const testFile = "/tmp/pwned";

      // Clean up any existing test file
      if (existsSync(testFile)) {
        unlinkSync(testFile);
      }

      // Call the function with malicious input
      const result = validateSHAExists(maliciousRef);

      // Behavior check: Verify no arbitrary commands were executed
      // If command injection occurred, /tmp/pwned would exist
      expect(existsSync(testFile)).toBe(false);

      // Pattern check: Verify execFileSync was called with array arguments
      expect(execFileSyncSpy).toHaveBeenCalledWith(
        "git",
        ["cat-file", "-t", maliciousRef],
        expect.objectContaining({ stdio: "ignore" })
      );

      // Verify the malicious string was passed as-is (not shell-expanded)
      const callArgs = execFileSyncSpy.mock.calls[0];
      const argsArray = callArgs[1];
      expect(argsArray).toEqual(["cat-file", "-t", maliciousRef]);
      expect(argsArray![2]).toBe(maliciousRef);

      // Verify the function returns false (invalid ref handled safely)
      expect(result).toBe(false);

      execFileSyncSpy.mockRestore();

      // Clean up just in case
      if (existsSync(testFile)) {
        unlinkSync(testFile);
      }
    });
  });
});
