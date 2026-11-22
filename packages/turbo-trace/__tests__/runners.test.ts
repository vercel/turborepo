import { spawnSync } from "node:child_process";
import {
  existsSync,
  mkdirSync,
  mkdtempSync,
  readFileSync,
  rmSync,
  writeFileSync
} from "node:fs";
import { dirname, join, resolve } from "node:path";
import { afterEach, beforeEach, describe, expect, it } from "@jest/globals";

// The real runners on a small repository. By default they are this package's
// own (one major each). `TURBO_TRACE_TEST_RUNNERS` names a directory whose
// `node_modules` holds another major and a link to this package
// (`scripts/test-runners.sh` makes one for every supported major).
const runners =
  process.env.TURBO_TRACE_TEST_RUNNERS ?? resolve(__dirname, "..");
const version = (runner: string): number[] | undefined => {
  const manifest = join(runners, "node_modules", runner, "package.json");
  return existsSync(manifest)
    ? JSON.parse(readFileSync(manifest, "utf8")).version.split(".").map(Number)
    : undefined;
};
const jestVersion = version("jest");
const vitestVersion = version("vitest");

const selection = {
  version: 1,
  runAll: false,
  runWhole: ["jest/whole/", "vitest/whole/"],
  files: ["jest/a.test.js", "vitest/a.test.mjs"],
  unselected: [
    "jest/b.test.js",
    "jest/projects/two/b.test.js",
    "jest/spread/t.test.js",
    "jest/whole/c.test.js",
    "vitest/(group)/[id]/d.test.mjs",
    "vitest/b.test.mjs",
    "vitest/projects/two/b.test.mjs",
    "vitest/src/b.test.mjs",
    "vitest/whole/c.test.mjs"
  ],
  total: 11
};

const jestTest =
  "test('runs', () => require('node:fs').appendFileSync(process.env.RAN, __filename + '\\n'));\n";
const vitestTest =
  "import { appendFileSync } from 'node:fs';\nimport { fileURLToPath } from 'node:url';\nimport { test } from 'vitest';\ntest('runs', () => appendFileSync(process.env.RAN, fileURLToPath(import.meta.url) + '\\n'));\n";
const jestConfig =
  "testEnvironment: 'node', testMatch: ['<rootDir>/**/*.test.js'], testPathIgnorePatterns: ['/projects/']";
const files: Record<string, string> = {
  "selection.json": JSON.stringify(selection),
  "jest/jest.config.js": `const { withTurboTrace } = require('@turbo/trace/jest');\nmodule.exports = withTurboTrace({ ${jestConfig} });\n`,
  // A package's configuration that spreads the root's.
  "jest/spread/jest.config.js":
    "module.exports = { ...require('../jest.config.js'), rootDir: __dirname };\n",
  "jest/esm.config.mjs": `import { withTurboTrace } from '@turbo/trace/jest';\nexport default withTurboTrace(async () => ({ ${jestConfig} }));\n`,
  "jest/own.config.js": `const { withTurboTrace } = require('@turbo/trace/jest');\nmodule.exports = withTurboTrace({ ${jestConfig}, filter: '<rootDir>/own-filter.js' });\n`,
  "jest/own-filter.js":
    "module.exports = async (tests) => ({ filtered: tests.filter((test) => !test.includes('unknown')).flatMap((test) => [test, { test }]) });\n",
  "jest/projects.config.js":
    "const { withTurboTrace } = require('@turbo/trace/jest');\nmodule.exports = withTurboTrace({ projects: ['one', 'two'].map((name) => ({ displayName: name, testEnvironment: 'node', rootDir: __dirname + '/projects/' + name })) });\n",
  "jest/handle.config.js":
    "const { withTurboTrace } = require('@turbo/trace/jest');\nmodule.exports = withTurboTrace({ testEnvironment: 'node', testMatch: ['<rootDir>/open.handle.js'] });\n",
  "jest/open.handle.js":
    "test('leaves a timer', () => { setInterval(() => {}, 1000); });\n",
  "vitest/vitest.config.mjs":
    "import { turboTrace } from '@turbo/trace/vitest';\nconst test = {};\nif (process.env.EXCLUDE) test.exclude = [process.env.EXCLUDE];\nif (process.env.DIR) test.dir = process.env.DIR;\nexport default { plugins: [turboTrace()], test };\n",
  "vitest/required.config.cjs":
    "const { turboTrace } = require('@turbo/trace/vitest');\nmodule.exports = { plugins: [turboTrace()] };\n",
  "vitest/projects.config.mjs":
    "import { turboTrace } from '@turbo/trace/vitest';\nexport default { test: { projects: ['one', 'two'].map((name) => ({ plugins: [turboTrace()], test: { name, root: 'projects/' + name } })) } };\n",
  ...Object.fromEntries(
    [
      "a",
      "b",
      "whole/c",
      "unknown",
      "spread/s",
      "spread/t",
      "projects/one/a",
      "projects/two/b"
    ].map((name) => [`jest/${name}.test.js`, jestTest])
  ),
  ...Object.fromEntries(
    [
      "a",
      "b",
      "whole/c",
      "unknown",
      "(group)/[id]/d",
      "(group)/i/d",
      "src/b",
      "node_modules/dep/e",
      "projects/one/a",
      "projects/two/b"
    ].map((name) => [`vitest/${name}.test.mjs`, vitestTest])
  )
};

let repo: string;

beforeEach(() => {
  repo = mkdtempSync(join(runners, ".trace-fixture-"));
  for (const [file, content] of Object.entries(files)) {
    mkdirSync(dirname(join(repo, file)), { recursive: true });
    writeFileSync(join(repo, file), content);
  }
});

afterEach(() => rmSync(repo, { recursive: true, force: true }));

/** Runs a runner in its directory of the repository: the test files that ran, and more. */
function run(
  runner: "jest" | "vitest",
  args: string[],
  env: Record<string, string> = { TURBO_TRACE_SELECTION: "selection.json" }
) {
  const set = Object.fromEntries(
    Object.entries(env).map(([key, value]) => [
      key,
      key.startsWith("TURBO_") ? join(repo, value) : value
    ])
  );
  const cwd = join(repo, runner);
  rmSync(join(repo, "ran"), { force: true });
  const result = spawnSync(join(runners, "node_modules/.bin", runner), args, {
    cwd,
    encoding: "utf8",
    env: {
      PATH: process.env.PATH,
      HOME: process.env.HOME,
      CI: "1",
      RAN: join(repo, "ran"),
      ...set
    },
    timeout: 60_000
  });
  const read = (file: string) =>
    existsSync(join(repo, file))
      ? readFileSync(join(repo, file), "utf8").trim().split("\n")
      : [];
  return {
    output: result.stdout + result.stderr,
    ran: read("ran")
      .map((file) => file.slice(cwd.length + 1))
      .sort(),
    report: read("report").map((line) => JSON.parse(line)),
    status: result.status
  };
}

(jestVersion ? describe : describe.skip)(
  `jest ${jestVersion?.join(".")}`,
  () => {
    const selected = [
      "a.test.js",
      "spread/s.test.js",
      "unknown.test.js",
      "whole/c.test.js"
    ];
    const every = [
      "a.test.js",
      "b.test.js",
      "spread/s.test.js",
      "spread/t.test.js",
      "unknown.test.js",
      "whole/c.test.js"
    ];

    it("runs the selected, the whole-directory and the unknown test files", () => {
      const { output, ran, report, status } = run("jest", [], {
        TURBO_TRACE_SELECTION: "selection.json",
        TURBO_TRACE_REPORT: "report"
      });
      expect({ ran, status }).toEqual({ ran: selected, status: 0 });
      expect(report).toEqual([
        { runner: "jest", root: "jest", ran: 4, total: 6 }
      ]);
      expect(output).toContain("turbo trace: running 4 of 6 test files");
    });

    it("runs everything without a selection, and says nothing", () => {
      const { output, ran, status } = run("jest", [], {});
      expect({ ran, status }).toEqual({ ran: every, status: 0 });
      expect(output).not.toContain("turbo trace");
      expect(output).not.toContain("Force exiting");
    });

    it("runs everything, with a warning, on a selection it cannot read", () => {
      const { output, ran, status } = run("jest", [], {
        TURBO_TRACE_SELECTION: "jest/a.test.js"
      });
      expect({ ran, status }).toEqual({ ran: every, status: 0 });
      expect(output).toContain(": running every test");
    });

    it("passes when the files that were asked for are all dropped", () => {
      expect(run("jest", ["b.test.js"])).toMatchObject({ ran: [], status: 0 });
      expect(run("jest", ["--listTests"]).output).not.toContain("b.test.js");
    });

    it("filters a configuration that spreads the wrapped one", () => {
      expect(run("jest", ["-c", "spread/jest.config.js"])).toMatchObject({
        ran: ["spread/s.test.js"],
        status: 0
      });
    });

    it("wraps an ES module configuration that is an async function", () => {
      expect(run("jest", ["-c", "esm.config.mjs"])).toMatchObject({
        ran: selected,
        status: 0
      });
    });

    it("leaves a configuration that has a filter to that filter, with a warning", () => {
      const { output, ran, status } = run("jest", ["-c", "own.config.js"]);
      expect({ ran, status }).toEqual({
        ran: every.filter((file) => !file.includes("unknown")),
        status: 0
      });
      expect(output).toContain("running every test it keeps");
    });

    it("filters every project of a wrapped configuration", () => {
      const { ran, report, status } = run(
        "jest",
        ["-c", "projects.config.js"],
        {
          TURBO_TRACE_SELECTION: "selection.json",
          TURBO_TRACE_REPORT: "report"
        }
      );
      expect({ ran, status }).toEqual({
        ran: ["projects/one/a.test.js"],
        status: 0
      });
      expect(report.map((line) => [line.ran, line.total]).sort()).toEqual([
        [0, 1],
        [1, 1]
      ]);
    });

    it("is the filter module itself with --filter", () => {
      const filter = join(
        runners,
        "node_modules/@turbo/trace/dist/jest/filter.js"
      );
      const args = [
        "--rootDir",
        ".",
        "--testMatch",
        "**/b.test.js",
        "--passWithNoTests",
        "--filter",
        existsSync(filter)
          ? filter
          : resolve(__dirname, "../dist/jest/filter.js")
      ];
      expect(run("jest", ["-c", "{}", ...args.slice(0, 5)], {})).toMatchObject({
        ran: ["b.test.js", "projects/two/b.test.js"]
      });
      expect(run("jest", ["-c", "{}", ...args])).toMatchObject({
        ran: [],
        status: 0
      });
    });

    it("exits with a test that leaves a handle open, which jest runs in its own process", () => {
      const { output, status } = run("jest", ["-c", "handle.config.js"]);
      expect(status).toBe(0);
      expect(output).toContain("Force exiting Jest");
    });
  }
);

(vitestVersion ? describe : describe.skip)(
  `vitest ${vitestVersion?.join(".")}`,
  () => {
    const [major, minor] = vitestVersion ?? [0, 0];
    const selected = [
      "(group)/i/d.test.mjs",
      "a.test.mjs",
      "unknown.test.mjs",
      "whole/c.test.mjs"
    ];
    const every = [
      "(group)/[id]/d.test.mjs",
      "(group)/i/d.test.mjs",
      "a.test.mjs",
      "b.test.mjs",
      "projects/one/a.test.mjs",
      "projects/two/b.test.mjs",
      "src/b.test.mjs",
      "unknown.test.mjs",
      "whole/c.test.mjs"
    ];
    const withProjects = [...selected, "projects/one/a.test.mjs"].sort();

    it("runs the selected, the whole-directory and the unknown test files", () => {
      const { output, ran, report, status } = run("vitest", ["run"], {
        TURBO_TRACE_SELECTION: "selection.json",
        TURBO_TRACE_REPORT: "report"
      });
      expect({ ran, status }).toEqual({ ran: withProjects, status: 0 });
      expect(report).toEqual([
        { runner: "vitest", root: "vitest", ran: 2, total: 6 }
      ]);
      expect(output).toContain("turbo trace: running 2 of 6 test files");
    });

    it("runs everything without a selection, and says nothing", () => {
      const { output, ran, status } = run("vitest", ["run"], {});
      expect({ ran, status }).toEqual({ ran: every, status: 0 });
      expect(output).not.toContain("turbo trace");
    });

    it("runs everything, with a warning, on a selection it cannot read", () => {
      const { output, ran, status } = run("vitest", ["run"], {
        TURBO_TRACE_SELECTION: "vitest/a.test.mjs"
      });
      expect({ ran, status }).toEqual({ ran: every, status: 0 });
      expect(output).toContain(": running every test");
    });

    it("passes when the files that were asked for are all dropped", () => {
      expect(run("vitest", ["run", "b.test"])).toMatchObject({
        ran: [],
        status: 0
      });
    });

    it("adds to the excludes of the configuration", () => {
      const env = {
        TURBO_TRACE_SELECTION: "selection.json",
        EXCLUDE: "**/unknown.test.mjs"
      };
      expect(run("vitest", ["run"], env)).toMatchObject({
        ran: [
          ...withProjects.filter((file) => !file.includes("unknown")),
          "node_modules/dep/e.test.mjs"
        ].sort(),
        status: 0
      });
    });

    it("drops below the directory the tests are looked for in", () => {
      const env = { TURBO_TRACE_SELECTION: "selection.json", DIR: "src" };
      expect(run("vitest", ["run"], env)).toMatchObject({ ran: [], status: 0 });
      expect(run("vitest", ["run"], { DIR: "src" })).toMatchObject({
        ran: ["src/b.test.mjs"],
        status: 0
      });
    });

    it("works in a configuration that is loaded with require", () => {
      expect(
        run("vitest", ["run", "--config", "required.config.cjs"])
      ).toMatchObject({ ran: withProjects, status: 0 });
    });

    it("filters each project that has the plugin", () => {
      // `test.projects` came with vitest 3.2.
      if (major < 3 || (major === 3 && minor < 2)) {
        return;
      }
      const args = ["run", "--config", "projects.config.mjs"];
      expect(run("vitest", args)).toMatchObject({
        ran: ["projects/one/a.test.mjs"],
        status: 0
      });
      // The plugin is in the projects only, and nothing is left to run.
      expect(run("vitest", [...args, "b.test"])).toMatchObject({
        ran: [],
        status: 0
      });
    });
  }
);
