import { describe, it, expect } from "vitest";
import { readFileSync } from "fs";
import { join } from "path";

describe("turbo.json environment configuration", () => {
  const turboConfig = JSON.parse(
    readFileSync(join(__dirname, "turbo.json"), "utf8")
  );

  it("should have globalEnv with CI", () => {
    expect(turboConfig.globalEnv).toBeDefined();
    expect(turboConfig.globalEnv).toContain("CI");
  });

  it("should have build task with env array containing NODE_ENV", () => {
    expect(turboConfig.tasks.build.env).toBeDefined();
    expect(turboConfig.tasks.build.env).toContain("NODE_ENV");
  });
});
