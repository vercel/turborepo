import { getTransformsForMigration } from "../src/commands/migrate/steps/getTransformsForMigration";

describe("get-transforms-for-migration", () => {
  test("ordering", () => {
    const results = getTransformsForMigration({
      fromVersion: "1.0.0",
      toVersion: "1.10.0",
    });

    expect(results.map((transform) => transform.name)).toEqual([
      "add-package-manager",
      "create-turbo-config",
      "migrate-env-var-dependencies",
      "set-default-outputs",
      "stabilize-env-mode",
      "transform-env-literals-to-wildcards",
    ]);
  });
});
