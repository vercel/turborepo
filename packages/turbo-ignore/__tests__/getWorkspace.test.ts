import { getWorkspace } from "../src/getWorkspace";
import { spyConsole, validateLogs } from "@turbo/test-utils";

describe("getWorkspace()", () => {
  const mockConsole = spyConsole();
  it("getWorkspace returns workspace from arg", async () => {
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

  it("getWorkspace returns workspace from package.json", async () => {
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

  it("getWorkspace used current directory if not specified", async () => {
    expect(getWorkspace({})).toEqual("turbo-ignore");
    expect(mockConsole.log).toHaveBeenCalledWith(
      "≫  ",
      'Inferred "turbo-ignore" as workspace from "package.json"'
    );
  });

  it("getWorkspace returns null when no arg is provided and package.json is missing name field", async () => {
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

  it("getWorkspace returns null when no arg is provided and package.json can be found", async () => {
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
