import { spyConsole, validateLogs } from "@turbo/test-utils";
import { getTurboVersion } from "../src/getTurboVersion";

describe("getWorkspace()", () => {
  const mockConsole = spyConsole();
  it("getTurboVersion returns turboVersion from arg", () => {
    expect(
      getTurboVersion(
        {
          turboVersion: "1.2.3",
        },
        "./__fixtures__/app"
      )
    ).toEqual("1.2.3");
    validateLogs(
      ['Using turbo version "1.2.3" from arguments'],
      mockConsole.log,
      { prefix: "≫  " }
    );
  });

  it("getTurboVersion returns version from package.json", () => {
    expect(getTurboVersion({}, "./__fixtures__/turbo_in_deps")).toEqual("^99");
    expect(mockConsole.log).toHaveBeenCalledWith(
      "≫  ",
      'Inferred turbo version "^99" from "package.json"'
    );
  });

  it("getTurboVersion infers ^2 if tasks in turbo.json", () => {
    expect(getTurboVersion({}, "./__fixtures__/no_turbo_deps")).toEqual("^2");
    expect(mockConsole.log).toHaveBeenCalledWith(
      "≫  ",
      'Inferred turbo version ^2 based on "tasks" in "turbo.json"'
    );
  });

  it("getTurboVersion infers ^1 if pipeline in turbo.json", () => {
    expect(getTurboVersion({}, "./__fixtures__/no_turbo_deps_v1")).toEqual(
      "^1"
    );
    expect(mockConsole.log).toHaveBeenCalledWith(
      "≫  ",
      'Inferred turbo version ^1 based on "pipeline" in "turbo.json"'
    );
  });

  it("getTurboVersion return null if no turbo.json", () => {
    expect(getTurboVersion({}, "./__fixtures__/app")).toEqual(null);
    expect(mockConsole.error).toHaveBeenCalledWith(
      "≫  ",
      '"__fixtures__/app/turbo.json" could not be read. turbo-ignore turbo version inference failed'
    );
  });

  it("getTurboVersion return null if no package.json", () => {
    expect(getTurboVersion({}, "./__fixtures__/no-app")).toEqual(null);
    expect(mockConsole.error).toHaveBeenCalledWith(
      "≫  ",
      '"__fixtures__/no-app/package.json" could not be read. turbo-ignore turbo version inference failed'
    );
  });

  it("getTurboVersion return null if invalid JSON", () => {
    expect(getTurboVersion({}, "./__fixtures__/invalid_turbo_json")).toEqual(
      null
    );
  });
});
