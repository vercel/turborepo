import {
  mkdtempSync,
  mkdirSync,
  rmSync,
  writeFileSync,
  existsSync
} from "node:fs";
import os from "node:os";
import path from "node:path";
import { afterAll, describe, expect, it } from "@jest/globals";
import { setupTestFixtures } from "@turbo/test-utils";

type Fixture = ReturnType<ReturnType<typeof setupTestFixtures>["useFixture"]>;

describe.each(["common", ""])("fixture isolation with test name %j", (test) => {
  const directory = mkdtempSync(path.join(os.tmpdir(), "turbo-fixtures-"));
  const source = path.join(directory, "__fixtures__", test, "example");
  mkdirSync(source, { recursive: true });
  writeFileSync(path.join(source, "index.js"), "export const value = 1;");

  afterAll(() => {
    rmSync(directory, { recursive: true, force: true });
  });

  let secondSuite: ReturnType<typeof setupTestFixtures>;
  let first: Fixture;
  let second: Fixture;

  describe("finishing suite", () => {
    const firstSuite = setupTestFixtures({ directory, test });

    it("creates overlapping fixtures for two suites", () => {
      first = firstSuite.useFixture({ fixture: "example" });
      second = secondSuite.useFixture({ fixture: "example" });
      expect(first.read("index.js")).toBe("export const value = 1;");
      expect(second.read("index.js")).toBe("export const value = 1;");
    });
  });

  describe("still-active suite", () => {
    secondSuite = setupTestFixtures({ directory, test });

    it("can read and write its fixture after the other suite finishes", () => {
      expect(existsSync(first.root)).toBe(false);
      expect(second.read("index.js")).toBe("export const value = 1;");
      second.write("index.js", "export const value = 2;");
      expect(second.read("index.js")).toBe("export const value = 2;");
    });
  });
});
