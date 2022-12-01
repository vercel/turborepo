import path from "path";
import findTurboConfig from "../../lib/utils/findTurboConfig";

describe("findTurboConfig", () => {
  it("it parses valid turbo.json file", () => {
    const cwd = path.join(__dirname, "..", "fixtures", "workspace");
    expect(findTurboConfig({ cwd })).toEqual({
      $schema: "https://turbo.build/schema.json",
      globalEnv: ["UNORDERED", "CI"],
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
});
