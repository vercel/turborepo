import { getWorkspace } from "../src/getWorkspace";
import { spyConsole } from "../src/test-utils";

describe("getWorkspace()", () => {
  const mockConsole = spyConsole();
  it("getWorkspace returns workspace from args", async () => {
    expect(
      getWorkspace({
        args: { workspace: "test-workspace", filterFallback: true },
        cwd: process.cwd(),
      })
    ).toEqual("test-workspace");
    expect(mockConsole.log).toHaveBeenCalledWith(
      "≫  ",
      "using provided test-workspace as workspace"
    );
  });

  it("getWorkspace returns workspace from package.json", async () => {
    expect(
      getWorkspace({
        args: { workspace: null, filterFallback: true },
        cwd: "./__fixtures__/app",
      })
    ).toEqual("test-app");
    expect(mockConsole.log).toHaveBeenCalledWith(
      "≫  ",
      'inferred "test-app" as workspace from "package.json"'
    );
  });

  it("getWorkspace returns null when no args is provided or package.json can be found", async () => {
    expect(
      getWorkspace({
        args: { workspace: null, filterFallback: true },
        cwd: "../",
      })
    ).toEqual(null);
    expect(mockConsole.error).toHaveBeenCalledWith(
      "≫  ",
      '"../package.json" could not be found.'
    );
  });
});
