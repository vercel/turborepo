import path from "node:path";
import fs from "node:fs";
import { Linter } from "eslint";
import {
  afterEach,
  beforeEach,
  describe,
  expect,
  it,
  jest
} from "@jest/globals";
import { setupTestFixtures } from "@turbo/test-utils";
import { RULES } from "../../../../lib/constants";
import rule, { clearCache } from "../../../../lib/rules/no-undeclared-env-vars";

const FIXTURE = "workspace-configs-reload";

const linter = new Linter();

interface FsSnapshot {
  configExistsSync: number;
  configStatSync: number;
  configReadFileSync: number;
}

function isTurboConfigPath(filePath: fs.PathLike | number): boolean {
  return (
    typeof filePath === "string" &&
    /turbo\.jsonc?$/.test(path.basename(filePath))
  );
}

/**
 * Count filesystem operations that touch turbo.json/turbo.jsonc files. ESLint
 * creates the rule once per source file, so these counters track exactly the
 * per-file validation work the rule performs on configs.
 */
function instrumentFs() {
  const counts: FsSnapshot = {
    configExistsSync: 0,
    configStatSync: 0,
    configReadFileSync: 0
  };
  const originals = {
    existsSync: fs.existsSync,
    statSync: fs.statSync,
    readFileSync: fs.readFileSync
  };
  // The module namespace types these functions as read-only, so swap them
  // out through a mutable view for the duration of a test.
  const mutableFs = fs as unknown as {
    existsSync: typeof fs.existsSync;
    statSync: typeof fs.statSync;
    readFileSync: typeof fs.readFileSync;
  };

  mutableFs.existsSync = ((
    ...args: Parameters<typeof fs.existsSync>
  ): ReturnType<typeof fs.existsSync> => {
    if (isTurboConfigPath(args[0])) {
      counts.configExistsSync++;
    }
    return originals.existsSync(...args);
  }) as typeof fs.existsSync;

  mutableFs.statSync = ((
    ...args: Parameters<typeof fs.statSync>
  ): ReturnType<typeof fs.statSync> => {
    if (isTurboConfigPath(args[0])) {
      counts.configStatSync++;
    }
    return originals.statSync(...args);
  }) as typeof fs.statSync;

  mutableFs.readFileSync = ((
    ...args: Parameters<typeof fs.readFileSync>
  ): ReturnType<typeof fs.readFileSync> => {
    if (isTurboConfigPath(args[0])) {
      counts.configReadFileSync++;
    }
    return originals.readFileSync(...args);
  }) as typeof fs.readFileSync;

  return {
    snapshot: (): FsSnapshot => ({ ...counts }),
    restore: () => {
      mutableFs.existsSync = originals.existsSync;
      mutableFs.statSync = originals.statSync;
      mutableFs.readFileSync = originals.readFileSync;
    }
  };
}

describe("turbo config cache in no-undeclared-env-vars", () => {
  const { useFixture } = setupTestFixtures({
    directory: path.join(__dirname, "../../../../")
  });

  // Controlled clock for the validation interval
  let mockNow: number;

  const lintFile = (
    filename: string,
    code: string,
    cwd: string
  ): Array<{ message: string }> =>
    linter.verify(
      code,
      {
        plugins: {
          turbo: { rules: { [RULES.noUndeclaredEnvVars]: rule } }
        },
        languageOptions: { ecmaVersion: 2020, sourceType: "module" },
        rules: {
          "turbo/no-undeclared-env-vars": ["error", { cwd }]
        }
      },
      { filename }
    );

  beforeEach(() => {
    clearCache();
    mockNow = 1_700_000_000_000;
    jest.spyOn(Date, "now").mockImplementation(() => mockNow);
  });

  afterEach(() => {
    jest.restoreAllMocks();
    clearCache();
  });

  it("does not re-validate turbo configs for every file in a batch", () => {
    const { root: cwd } = useFixture({ fixture: FIXTURE });
    const files = [
      path.join(cwd, "apps/web/index.js"),
      path.join(cwd, "apps/docs/index.js"),
      path.join(cwd, "packages/ui/index.js")
    ];
    const code = "const { ENV_1 } = process.env;";

    const instrument = instrumentFs();
    try {
      // The first file initializes the project cache
      lintFile(files[0], code, cwd);
      const afterFirstFile = instrument.snapshot();

      // ESLint creates the rule once per source file
      for (let i = 0; i < 24; i++) {
        lintFile(files[i % files.length], code, cwd);
      }
      const afterBatch = instrument.snapshot();

      // A batch of unchanged files performs no per-file config validation
      expect(
        afterBatch.configExistsSync - afterFirstFile.configExistsSync
      ).toBe(0);
      expect(afterBatch.configStatSync - afterFirstFile.configStatSync).toBe(0);
      expect(
        afterBatch.configReadFileSync - afterFirstFile.configReadFileSync
      ).toBe(0);
    } finally {
      instrument.restore();
    }
  });

  it("validates via stat metadata without reading config contents", () => {
    const { root: cwd } = useFixture({ fixture: FIXTURE });
    const webFile = path.join(cwd, "apps/web/index.js");
    const code = "const { ENV_2 } = process.env;";

    const instrument = instrumentFs();
    try {
      lintFile(webFile, code, cwd);
      const afterInit = instrument.snapshot();

      // A lint run after the validation interval (e.g. a later editor pass)
      // sweeps the configs, but nothing changed so nothing is re-read
      mockNow += 1_500;
      lintFile(webFile, code, cwd);
      const afterValidation = instrument.snapshot();

      expect(
        afterValidation.configStatSync - afterInit.configStatSync
      ).toBeGreaterThan(0);
      expect(
        afterValidation.configReadFileSync - afterInit.configReadFileSync
      ).toBe(0);

      // Findings are still correct
      expect(lintFile(webFile, code, cwd)).toEqual([]);
    } finally {
      instrument.restore();
    }
  });

  it("serves cached configs within the interval and picks up same-size edits on the next validation", () => {
    const { root: cwd } = useFixture({ fixture: FIXTURE });
    const webFile = path.join(cwd, "apps/web/index.js");
    const webConfigPath = path.join(cwd, "apps/web/turbo.json");
    const original = fs.readFileSync(webConfigPath, "utf8");

    try {
      // ENV_2 is declared by apps/web/turbo.json
      expect(lintFile(webFile, "const { ENV_2 } = process.env;", cwd)).toEqual(
        []
      );

      // Same-size content change: ENV_2 -> ENV_4 (size is unchanged, mtime is not)
      const edited = original.replace('"ENV_2"', '"ENV_4"');
      expect(edited).not.toBe(original);
      expect(edited).toHaveLength(original.length);
      fs.writeFileSync(webConfigPath, edited);

      // A lint run within the interval is served from the cached project
      expect(lintFile(webFile, "const { ENV_4 } = process.env;", cwd)).toEqual([
        expect.objectContaining({
          message: expect.stringContaining("ENV_4 is not listed")
        })
      ]);

      // The next lint run after the interval reloads the edited config
      mockNow += 1_500;
      expect(lintFile(webFile, "const { ENV_4 } = process.env;", cwd)).toEqual(
        []
      );
      expect(lintFile(webFile, "const { ENV_2 } = process.env;", cwd)).toEqual([
        expect.objectContaining({
          message: expect.stringContaining("ENV_2 is not listed")
        })
      ]);
    } finally {
      fs.writeFileSync(webConfigPath, original);
    }
  });

  it("detects removed and newly added workspace configs", () => {
    const { root: cwd } = useFixture({ fixture: FIXTURE });
    const docsFile = path.join(cwd, "apps/docs/index.js");
    const docsConfigPath = path.join(cwd, "apps/docs/turbo.json");
    const original = fs.readFileSync(docsConfigPath, "utf8");

    try {
      // ENV_3 is declared by apps/docs/turbo.json
      expect(lintFile(docsFile, "const { ENV_3 } = process.env;", cwd)).toEqual(
        []
      );

      // Remove the workspace config
      fs.rmSync(docsConfigPath);
      mockNow += 1_500;
      expect(lintFile(docsFile, "const { ENV_3 } = process.env;", cwd)).toEqual(
        [
          expect.objectContaining({
            message: "ENV_3 is not listed as a dependency in root turbo.json"
          })
        ]
      );

      // Re-add a config declaring a different variable
      fs.writeFileSync(docsConfigPath, original.replace('"ENV_3"', '"ENV_5"'));
      mockNow += 1_500;
      expect(lintFile(docsFile, "const { ENV_5 } = process.env;", cwd)).toEqual(
        []
      );
      expect(lintFile(docsFile, "const { ENV_3 } = process.env;", cwd)).toEqual(
        [
          expect.objectContaining({
            message: expect.stringContaining("ENV_3 is not listed")
          })
        ]
      );
    } finally {
      fs.writeFileSync(docsConfigPath, original);
    }
  });

  it("detects edits to the root config on the next validation", () => {
    const { root: cwd } = useFixture({ fixture: FIXTURE });
    const docsFile = path.join(cwd, "apps/docs/index.js");
    const rootConfigPath = path.join(cwd, "turbo.json");
    const original = fs.readFileSync(rootConfigPath, "utf8");

    try {
      // CI is declared by the root turbo.json globalEnv
      expect(lintFile(docsFile, "const { CI } = process.env;", cwd)).toEqual(
        []
      );

      const edited = original.replace('"CI"', '"CI_2"');
      expect(edited).not.toBe(original);
      fs.writeFileSync(rootConfigPath, edited);

      mockNow += 1_500;
      expect(lintFile(docsFile, "const { CI_2 } = process.env;", cwd)).toEqual(
        []
      );
      expect(lintFile(docsFile, "const { CI } = process.env;", cwd)).toEqual([
        expect.objectContaining({
          message: expect.stringContaining("CI is not listed")
        })
      ]);
    } finally {
      fs.writeFileSync(rootConfigPath, original);
    }
  });

  it("produces identical findings with a warm cache as with a cold cache", () => {
    const { root: cwd } = useFixture({ fixture: FIXTURE });
    const files = [
      {
        file: path.join(cwd, "apps/web/index.js"),
        code: "const { ENV_2, ENV_9 } = process.env;"
      },
      {
        file: path.join(cwd, "apps/docs/index.js"),
        code: "const { ENV_3, ENV_9 } = process.env;"
      },
      {
        file: path.join(cwd, "packages/ui/index.js"),
        code: "const { IS_SERVER, ENV_9 } = process.env;"
      }
    ];

    // Warm: one shared project cache across the whole batch
    const warm = files.map(({ file, code }) => lintFile(file, code, cwd));

    // Cold: fresh caches for every file
    const cold = files.map(({ file, code }) => {
      clearCache();
      mockNow += 1_500;
      return lintFile(file, code, cwd);
    });

    expect(warm).toEqual(cold);
  });
});
