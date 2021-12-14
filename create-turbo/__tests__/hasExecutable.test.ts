import path from "path";

import { hasExecutable } from "../src/hasExecutable";

const baseDirectory = path.join(__dirname, "__data__");

describe("hasExecutable", () => {
  test("works with process.env.PATH and process.env.PATHEXT", async () => {
    expect(await hasExecutable("node")).toBeTruthy();
  });

  test("returns true, if the executable is present", async () => {
    expect(
      await hasExecutable("b", [path.resolve(baseDirectory, "a")])
    ).toBeTruthy();
  });

  test("returns false, if the executable is not present", async () => {
    expect(
      await hasExecutable("c", [path.resolve(baseDirectory, "c")])
    ).toBeFalsy();
  });

  test("works with extensions", async () => {
    expect(
      await hasExecutable("e", [path.resolve(baseDirectory, "d")], [".command"])
    ).toBeTruthy();
  });
});
