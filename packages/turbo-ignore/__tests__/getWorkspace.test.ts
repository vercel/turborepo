import { spyConsole, validateLogs } from "@turbo/test-utils";
import { getWorkspace } from "../src/getWorkspace";

describe("getWorkspace()", () => {
  const mockConsole = spyConsole();
  it("getWorkspace returns workspace from arg", () => {
    expect(
      getWorkspace({
        workspace: "test-workspace",
      })
    ).toEqual("test-workspace");
    validateLogs(
      ['Using "test-workspace" as workspace from arguments'],
      mockConsole.log,
      { prefix: "≫  " }
    );
  });

  it("getWorkspace returns workspace from package.json", () => {
    expect(
      getWorkspace({
        directory: "./__fixtures__/app",
      })
    ).toEqual("test-app");
    expect(mockConsole.log).toHaveBeenCalledWith(
      "≫  ",
      'Inferred "test-app" as workspace from "package.json"'
    );
  });

  it("getWorkspace used current directory if not specified", () => {
    expect(getWorkspace({})).toEqual("turbo-ignore");
    expect(mockConsole.log).toHaveBeenCalledWith(
      "≫  ",
      'Inferred "turbo-ignore" as workspace from "package.json"'
    );
  });

  it("getWorkspace returns null when no arg is provided and package.json is missing name field", () => {
    expect(
      getWorkspace({
        directory: "./__fixtures__/invalid-app",
      })
    ).toEqual(null);
    expect(mockConsole.error).toHaveBeenCalledWith(
      "≫  ",
      '"__fixtures__/invalid-app/package.json" is missing the "name" field (required).'
    );
  });

  it("getWorkspace returns null when no arg is provided and package.json can be found", () => {
    expect(
      getWorkspace({
        directory: "./__fixtures__/no-app",
      })
    ).toEqual(null);
    expect(mockConsole.error).toHaveBeenCalledWith(
      "≫  ",
      '"__fixtures__/no-app/package.json" could not be found. turbo-ignore inferencing failed'
    );
  });
});
