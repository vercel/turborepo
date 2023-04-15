import { getTask } from "../src/getTask";
import { spyConsole, validateLogs } from "@turbo/test-utils";

describe("getWorkspace()", () => {
  const mockConsole = spyConsole();
  it("getTask defaults to build", async () => {
    expect(getTask({})).toEqual("build");
    validateLogs(
      ['Using "build" as the task as it was unspecified'],
      mockConsole.log,
      { prefix: "≫  " }
    );
  });

  it("getTask returns a quoted task if user-supplied", async () => {
    expect(
      getTask({
        task: "workspace#task",
      })
    ).toEqual(`"workspace#task"`);
    validateLogs(
      ['Using "workspace#task" as the task from the arguments'],
      mockConsole.log,
      { prefix: "≫  " }
    );
  });
});
