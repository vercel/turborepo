import { describe, it, expect } from "vitest";
import { readFileSync, existsSync } from "fs";
import { join } from "path";

describe("new config package", () => {
  const configPkgPath = join(__dirname, "packages/config");

  it("should have config package directory", () => {
    expect(existsSync(configPkgPath)).toBe(true);
  });

  it("should have config package.json with correct name", () => {
    const pkgJson = JSON.parse(
      readFileSync(join(configPkgPath, "package.json"), "utf8")
    );
    expect(pkgJson.name).toBe("@repo/config");
  });

  it("should have utils depending on config", () => {
    const utilsPkgJson = JSON.parse(
      readFileSync(join(__dirname, "packages/utils/package.json"), "utf8")
    );
    expect(utilsPkgJson.dependencies).toBeDefined();
    expect(utilsPkgJson.dependencies["@repo/config"]).toBeDefined();
  });
});
