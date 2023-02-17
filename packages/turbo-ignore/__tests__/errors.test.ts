import { shouldWarn, NON_FATAL_ERRORS } from "../src/errors";

describe("shouldWarn()", () => {
  it("it detects errors when packageManager is missing", async () => {
    const result = shouldWarn({
      err: `run failed: We did not detect an in-use package manager for your project. Please set the "packageManager" property in your root package.json (https://nodejs.org/api/packages.html#packagemanager) or run \`npx @turbo/codemod add-package-manager\` in the root of your monorepo.`,
    });
    expect(result.code).toBe("NO_PACKAGE_MANAGER");
    expect(result.level).toBe("warn");
    expect(result.message).toBe(NON_FATAL_ERRORS.NO_PACKAGE_MANAGER.message);
  });

  it("it detects errors when yarn lockfile is missing", async () => {
    const result = shouldWarn({
      err: `* reading yarn.lock: open /test/../yarn.lock: no such file or directory`,
    });
    expect(result.code).toBe("MISSING_LOCKFILE");
    expect(result.level).toBe("warn");
    expect(result.message).toBe(NON_FATAL_ERRORS.MISSING_LOCKFILE.message);
  });

  it("it detects errors when pnpm lockfile is missing", async () => {
    const result = shouldWarn({
      err: `* reading pnpm-lock.yaml: open /test/../pnpm-lock.yaml: no such file or directory`,
    });
    expect(result.code).toBe("MISSING_LOCKFILE");
    expect(result.level).toBe("warn");
    expect(result.message).toBe(NON_FATAL_ERRORS.MISSING_LOCKFILE.message);
  });

  it("it detects errors when npm lockfile is missing", async () => {
    const result = shouldWarn({
      err: `* reading package-lock.json: open /test/../package-lock.json: no such file or directory`,
    });
    expect(result.code).toBe("MISSING_LOCKFILE");
    expect(result.level).toBe("warn");
    expect(result.message).toBe(NON_FATAL_ERRORS.MISSING_LOCKFILE.message);
  });

  it("it returns unknown errors", async () => {
    const result = shouldWarn({ err: `something bad happened` });
    expect(result.code).toBe("UNKNOWN_ERROR");
    expect(result.level).toBe("error");
    expect(result.message).toBe(`something bad happened`);
  });
});
