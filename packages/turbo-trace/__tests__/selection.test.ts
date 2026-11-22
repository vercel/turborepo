import {
  mkdirSync,
  mkdtempSync,
  readFileSync,
  realpathSync,
  writeFileSync
} from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import {
  afterEach,
  beforeEach,
  describe,
  expect,
  it,
  jest
} from "@jest/globals";
import { filter as filterPath, withTurboTrace } from "../src/jest";
import filter from "../src/jest/filter";
import { loadSelection } from "../src/selection";
import { turboTrace } from "../src/vitest";

const base = {
  version: 1,
  runAll: false,
  runWhole: ["packages/whole/"],
  files: ["packages/a/a.test.js"],
  unselected: ["packages/a/b.test.js", "packages/whole/c.test.js"],
  total: 3
};

let repo: string;
let stderr: jest.SpiedFunction<typeof process.stderr.write>;
let count = 0;

/** Writes a selection (each one in a file of its own) and points the env at it. */
function select(doc: unknown): void {
  count += 1;
  const path = join(repo, `selection-${count}.json`);
  writeFileSync(path, typeof doc === "string" ? doc : JSON.stringify(doc));
  process.env.TURBO_TRACE_SELECTION = path;
}

const paths = (...files: string[]) => files.map((file) => join(repo, file));
const all = () =>
  paths(
    "packages/a/a.test.js",
    "packages/a/b.test.js",
    "packages/whole/c.test.js",
    "packages/a/new.test.js"
  );
const kept = async (tests: string[]) =>
  (await filter(tests)).filtered.filter((entry) => typeof entry === "string");
const warnings = () =>
  stderr.mock.calls
    .map(([line]) => String(line))
    .filter((line) => line.includes("running every test"));

beforeEach(() => {
  repo = realpathSync(mkdtempSync(join(tmpdir(), "turbo-trace-")));
  for (const file of [...base.files, ...base.unselected]) {
    mkdirSync(join(repo, file, ".."), { recursive: true });
    writeFileSync(join(repo, file), "");
  }
  jest.spyOn(process, "cwd").mockReturnValue(join(repo, "packages/a"));
  stderr = jest.spyOn(process.stderr, "write").mockReturnValue(true);
});

afterEach(() => {
  delete process.env.TURBO_TRACE_SELECTION;
  delete process.env.TURBO_TRACE_REPORT;
  jest.restoreAllMocks();
});

describe("the selection", () => {
  it("is absent without the variable, and with an empty one", () => {
    expect(loadSelection()).toBeUndefined();
    process.env.TURBO_TRACE_SELECTION = "";
    expect(loadSelection()).toBeUndefined();
    expect(stderr).not.toHaveBeenCalled();
  });

  it("drops the unselected files only: not a whole directory's, not an unknown one", async () => {
    select(base);
    await expect(kept(all())).resolves.toEqual(
      paths(
        "packages/a/a.test.js",
        "packages/whole/c.test.js",
        "packages/a/new.test.js"
      )
    );
    expect(warnings()).toEqual([]);
  });

  it("keeps a file outside the repository", async () => {
    select(base);
    const outside = [join(repo, "..", "packages/a/b.test.js")];
    await expect(kept(outside)).resolves.toEqual(outside);
  });

  it.each([
    ["not JSON", "{"],
    ["null", "null"],
    ["another version", { ...base, version: 2 }],
    ["no unselected list", { ...base, unselected: undefined }],
    ["a list that holds no strings", { ...base, runWhole: [1] }]
  ])(
    "runs everything, with one warning, for a document that is %s",
    async (_, doc) => {
      select(doc);
      await expect(kept(all())).resolves.toEqual(all());
      await expect(kept(all())).resolves.toEqual(all());
      expect(warnings()).toHaveLength(1);
    }
  );

  it("runs everything, with a warning, for a file that is missing", async () => {
    process.env.TURBO_TRACE_SELECTION = join(repo, "missing.json");
    await expect(kept(all())).resolves.toEqual(all());
    expect(warnings()).toHaveLength(1);
  });

  it.each([true, "false", undefined])(
    "runs everything when runAll is %p",
    async (runAll) => {
      select({ ...base, runAll });
      await expect(kept(all())).resolves.toEqual(all());
      expect(warnings()).toEqual([]);
    }
  );

  it("runs everything below the root package's whole directory", async () => {
    select({ ...base, runWhole: [""] });
    await expect(kept(all())).resolves.toEqual(all());
  });

  it("runs everything, with a warning, when its files are not around the working directory", async () => {
    select(base);
    jest.spyOn(process, "cwd").mockReturnValue(tmpdir());
    await expect(kept(all())).resolves.toEqual(all());
    expect(warnings()).toHaveLength(1);
  });

  it("takes the outermost directory that holds its files as the root", async () => {
    // A copy of the repository inside it: its paths are not the selection's.
    const copy = join(repo, "copy/packages/a");
    mkdirSync(copy, { recursive: true });
    writeFileSync(join(copy, "b.test.js"), "");
    jest.spyOn(process, "cwd").mockReturnValue(copy);
    select(base);
    const tests = [join(copy, "b.test.js"), join(repo, "packages/a/b.test.js")];
    await expect(kept(tests)).resolves.toEqual(tests.slice(0, 1));
  });

  it("appends one record to the report for each call", async () => {
    process.env.TURBO_TRACE_REPORT = join(repo, "report.jsonl");
    select(base);
    await filter(all());
    await filter(all().slice(0, 1));
    const lines = readFileSync(join(repo, "report.jsonl"), "utf8")
      .trim()
      .split("\n");
    expect(lines.map((line) => JSON.parse(line))).toEqual([
      { runner: "jest", root: "packages/a", ran: 3, total: 4 },
      { runner: "jest", root: "packages/a", ran: 1, total: 1 }
    ]);
    expect(stderr).toHaveBeenCalledWith(
      "turbo trace: running 3 of 4 test files\n"
    );
  });
});

describe("the jest filter", () => {
  it("answers in the forms of jest 27 to 29 and of jest 30", async () => {
    select(base);
    const [a] = all();
    await expect(filter([a])).resolves.toEqual({ filtered: [a, { test: a }] });
  });
});

describe("withTurboTrace", () => {
  const config = { testEnvironment: "node" };
  const wrapped = {
    ...config,
    filter: filterPath,
    forceExit: true,
    passWithNoTests: true
  };

  it("returns what it is given without a selection", () => {
    const fn = () => config;
    const promise = Promise.resolve(config);
    expect(withTurboTrace(config)).toBe(config);
    expect(withTurboTrace(fn)).toBe(fn);
    expect(withTurboTrace(promise)).toBe(promise);
  });

  it("gives an object for an object, with the filter as an absolute path", () => {
    select(base);
    expect(withTurboTrace(config)).toEqual(wrapped);
    expect(Object.getPrototypeOf(withTurboTrace(config))).toBe(
      Object.prototype
    );
    expect(require(filterPath).default ?? require(filterPath)).toBe(filter);
  });

  it("gives a function for a function and a promise for a promise", async () => {
    select(base);
    await expect(withTurboTrace(() => config)()).resolves.toEqual(wrapped);
    await expect(withTurboTrace(async () => config)()).resolves.toEqual(
      wrapped
    );
    await expect(withTurboTrace(Promise.resolve(config))).resolves.toEqual(
      wrapped
    );
  });

  it("leaves a configuration that has a filter of its own, with a warning", () => {
    select(base);
    const own = { ...config, filter: "<rootDir>/own-filter.js" };
    expect(withTurboTrace(own)).toBe(own);
    expect(warnings()).toHaveLength(1);
    // Its own filter is none: a configuration that spreads a wrapped one.
    expect(withTurboTrace(wrapped)).toEqual(wrapped);
    expect(warnings()).toHaveLength(1);
  });
});

describe("the vitest plugin", () => {
  it("does nothing outside vitest, and nothing without a selection", () => {
    const config = { root: join(repo, "packages/a"), test: {} };
    select(base);
    turboTrace().config(config);
    delete process.env.TURBO_TRACE_SELECTION;
    process.env.VITEST = "true";
    turboTrace().config(config);
    delete process.env.VITEST;
    expect(config).toEqual({ root: join(repo, "packages/a"), test: {} });
    expect(stderr).not.toHaveBeenCalled();
  });

  it("drops through the resolved configuration when the project has no vitest of its own", () => {
    process.env.VITEST = "true";
    process.env.TURBO_TRACE_REPORT = join(repo, "report.jsonl");
    select({
      ...base,
      unselected: [...base.unselected, "packages/a/[id]/(x).test.js"]
    });
    const plugin = turboTrace();
    const config: { root: string; test: Record<string, unknown> } = {
      root: join(repo, "packages/a"),
      test: {}
    };
    plugin.config(config);
    expect(config.test).toEqual({ passWithNoTests: true });
    const resolved = {
      dir: "",
      exclude: ["**/node_modules/**"],
      passWithNoTests: true,
      root: join(repo, "packages/a")
    };
    const run: { passWithNoTests?: boolean } = {};
    plugin.configureVitest({
      project: { config: resolved },
      vitest: { config: run }
    });
    delete process.env.VITEST;
    // The run passes without tests, whichever project the plugin is in.
    expect(run).toEqual({ passWithNoTests: true });
    expect(resolved.exclude).toEqual([
      "**/node_modules/**",
      "b.test.js",
      "\\[id\\]/\\(x\\).test.js"
    ]);
    expect(
      JSON.parse(readFileSync(join(repo, "report.jsonl"), "utf8"))
    ).toEqual({ runner: "vitest", root: "packages/a", ran: 1, total: 3 });
  });
});
