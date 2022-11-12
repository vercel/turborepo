import parseArgs, { help } from "../src/args";
import pkg from "../package.json";
import { spyConsole, spyExit } from "./test-utils";

describe("parseArgs()", () => {
  const mockConsole = spyConsole();
  const mockExit = spyExit();

  it("does not throw with no args", async () => {
    const result = parseArgs({ argv: [] });
    expect(result.workspace).toBe(null);
    expect(result.filterFallback).toBe(true);
  });

  it("outputs help text (--help)", async () => {
    parseArgs({ argv: ["--help"] });
    expect(mockExit.exit).toHaveBeenCalledWith(0);
    expect(mockConsole.log).toHaveBeenCalledWith(help);
  });

  it("outputs help text (-h)", async () => {
    parseArgs({ argv: ["-h"] });
    expect(mockExit.exit).toHaveBeenCalledWith(0);
    expect(mockConsole.log).toHaveBeenCalledWith(help);
  });

  it("outputs version text (--version)", async () => {
    parseArgs({ argv: ["--version"] });
    expect(mockExit.exit).toHaveBeenCalledWith(0);
    expect(mockConsole.log).toHaveBeenCalledWith(pkg.version);
  });

  it("outputs version text (-v)", async () => {
    parseArgs({ argv: ["-v"] });
    expect(mockExit.exit).toHaveBeenCalledWith(0);
    expect(mockConsole.log).toHaveBeenCalledWith(pkg.version);
  });

  it("correctly finds workspace", async () => {
    const result = parseArgs({ argv: ["this-workspace"] });
    expect(result.workspace).toBe("this-workspace");
    expect(result.filterFallback).toBe(true);
    expect(mockExit.exit).toHaveBeenCalledTimes(0);
  });

  it("correctly finds fallback", async () => {
    const result = parseArgs({ argv: ["--filter-fallback=false"] });
    expect(result.workspace).toBe(null);
    expect(result.filterFallback).toBe(false);
    expect(mockExit.exit).toHaveBeenCalledTimes(0);
  });

  it("correctly finds fallback and workspace", async () => {
    const result = parseArgs({
      argv: ["this-workspace", "--filter-fallback=false"],
    });
    expect(result.workspace).toBe("this-workspace");
    expect(result.filterFallback).toBe(false);
    expect(mockExit.exit).toHaveBeenCalledTimes(0);
  });
});
