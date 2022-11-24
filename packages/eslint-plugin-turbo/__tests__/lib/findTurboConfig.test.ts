import path from "path";
import findTurboConfig from "../../lib/utils/findTurboConfig";

test("Should parse valid turbo.json", () => {
  const cwd = path.resolve(__dirname, "./fixtures/workspace");
  expect(findTurboConfig({ cwd })).toEqual({
    $schema: "https://turbo.build/schema.json",
    pipeline: {
      build: {
        dependsOn: ["^build"],
      },
      test: {
        dependsOn: ["build"],
        outputs: [],
        inputs: [
          "src/**/*.tsx",
          "src/**/*.ts",
          "test/**/*.ts",
          "test/**/*.tsx",
        ],
      },
      lint: {
        outputs: [],
      },
      deploy: {
        dependsOn: ["build", "test", "lint"],
        outputs: [],
      },
    },
  });
});
