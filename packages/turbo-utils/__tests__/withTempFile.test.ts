import fs from "node:fs";
import { withTempFile } from "../src/withTempFile";

describe("withTempFile", () => {
  it("should create a writable file", () => {
    const contents = withTempFile((path) => {
      fs.writeFileSync(path, JSON.stringify({ data: "foo" }), {
        encoding: "utf8",
      });
      return fs.readFileSync(path, { encoding: "utf8" });
    });
    expect(contents).toEqual('{"data":"foo"}');
  });

  it("should clean up on task success", () => {
    const tempPath = withTempFile((path) => {
      return path;
    });
    expect(fs.existsSync(tempPath)).toEqual(false);
  });

  it("should clean up on task error", () => {
    let tempPath = "";
    try {
      withTempFile((path) => {
        tempPath = path;
        throw new Error("task error");
      });
    } catch {
      /* ignore error */
    }

    expect(tempPath).not.toEqual("");
    expect(fs.existsSync(tempPath)).toEqual(false);
  });
});
