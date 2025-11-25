// eslint-disable-next-line camelcase -- This is a test file
import child_process from "node:child_process";
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
    let mockExecFileSync: ReturnType<typeof jest.spyOn>;

    beforeEach(() => {
      mockExecFileSync = jest.spyOn(child_process, "execFileSync");
    });

    afterEach(() => {
      mockExecFileSync.mockRestore();
    });

    it("uses execFileSync with array arguments to prevent command injection", () => {
      const maliciousRef = "HEAD]; touch /tmp/pwned; echo [";
      mockExecFileSync.mockReturnValue("commit");

      const result = validateSHAExists(maliciousRef);

      expect(result).toBe(true);
      expect(mockExecFileSync).toHaveBeenCalledWith(
        "git",
        ["cat-file", "-t", maliciousRef],
        expect.objectContaining({ stdio: "ignore" })
      );
    });

    it("handles normal refs correctly", () => {
      const normalRef = "HEAD^";
      mockExecFileSync.mockReturnValue("commit");

      const result = validateSHAExists(normalRef);

      expect(result).toBe(true);
      expect(mockExecFileSync).toHaveBeenCalledWith(
        "git",
        ["cat-file", "-t", normalRef],
        expect.anything()
      );
    });

    it("returns false when git command fails", () => {
      const ref = "invalid-ref";
      mockExecFileSync.mockImplementation(() => {
        throw new Error("fatal: Not a valid object name");
      });

      const result = validateSHAExists(ref);

      expect(result).toBe(false);
    });
  });
});
