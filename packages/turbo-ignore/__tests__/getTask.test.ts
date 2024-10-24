import { spyConsole } from "@turbo/test-utils";
import { describe, it, expect } from "@jest/globals";
import { getTask } from "../src/getTask";

describe("getWorkspace()", () => {
  const mockConsole = spyConsole();
  it("getTask defaults to build", () => {
    expect(getTask({})).toEqual("build");
    expect(mockConsole.log).toHaveBeenNthCalledWith(
      1,
      "≫  ",
      'Using "build" as the task as it was unspecified'
    );
  });

  it("getTask returns a quoted task if user-supplied", () => {
    expect(
      getTask({
        task: "workspace#task",
      })
    ).toEqual(`"workspace#task"`);

    expect(mockConsole.log).toHaveBeenNthCalledWith(
      1,
      "≫  ",
      'Using "workspace#task" as the task from the arguments'
    );
  });
});
