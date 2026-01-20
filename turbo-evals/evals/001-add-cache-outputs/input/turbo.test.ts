import { describe, it, expect } from "vitest";
import { readFileSync } from "fs";
import { join } from "path";

describe("turbo.json configuration", () => {
  const turboConfig = JSON.parse(
    readFileSync(join(__dirname, "turbo.json"), "utf8")
  );

  it("should have test task with coverage outputs", () => {
    expect(turboConfig.tasks.test).toBeDefined();
    expect(turboConfig.tasks.test.outputs).toBeDefined();
    expect(turboConfig.tasks.test.outputs).toContain("coverage/**");
  });
});
