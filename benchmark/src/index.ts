import cp from "child_process";
import fs from "fs";
import fse from "fs-extra";
import path from "path";
import ndjson from "ndjson";

const REPO_ROOT = "large-monorepo";
const REPO_ORIGIN = "https://github.com/gsoltis/large-monorepo.git";
const REPO_PATH = path.join(process.cwd(), REPO_ROOT);
const REPETITIONS = 5;

const DEFAULT_EXEC_OPTS = { stdio: "ignore" as const, cwd: REPO_PATH };
const TURBO_BIN = path.resolve(path.join("..", "target", "release", "turbo"));
const DEFAULT_CACHE_PATH = path.join(
  REPO_PATH,
  "node_modules",
  ".cache",
  "turbo"
);
const ALT_CACHE_PATH = path.join(
  REPO_PATH,
  "node_modules",
  ".cache",
  "turbo-benchmark"
);

type Timing = number;

type Benchmark = {
  name: string;
  unit: string;
  value: number;
  range?: string;
  extra?: string;
};

type TBirdEvent = {
  commitSha: string;
  commitTimestamp: Date;
  platform: string;
  benchmark: string;
  durationMs: number;
};

function setup(): void {
  // Clone repo if it doesn't exist, run clean
  if (fs.existsSync(REPO_ROOT)) {
    // reset the repo, remove all changed or untracked files
    cp.execSync(
      `cd ${REPO_ROOT} && git reset --hard HEAD && git clean -f -d -X`,
      {
        stdio: "inherit",
      }
    );
  } else {
    cp.execSync(`git clone ${REPO_ORIGIN}`, { stdio: "ignore" });
  }

  // Run install so we aren't benchmarking node_modules ...

  cp.execSync("yarn install", DEFAULT_EXEC_OPTS);
}

function cleanTurboCache(): void {
  if (fs.existsSync(DEFAULT_CACHE_PATH)) {
    console.log("clearing cache");
    fs.rmSync(DEFAULT_CACHE_PATH, { recursive: true });
  }
}

function cleanBuild(): Timing[] {
  const timings: Timing[] = [];
  const isLocal = process.argv[process.argv.length - 1] == "--local";
  // We aren't really benchmarking this one, it OOMs if run in full parallel
  // on GH actions
  const repetitions = isLocal ? REPETITIONS : 1;
  const concurrency = isLocal ? "" : " --concurrency=1";
  for (let i = 0; i < repetitions; i++) {
    // clean first, we'll leave the cache in place for subsequent builds
    cleanTurboCache();
    const start = new Date().getTime();
    cp.execSync(`${TURBO_BIN} run build${concurrency}`, DEFAULT_EXEC_OPTS);
    const end = new Date().getTime();
    const timing = end - start;
    timings.push(timing);
  }
  return timings;
}

function cachedBuild(): Timing[] {
  const timings: Timing[] = [];
  for (let i = 0; i < REPETITIONS; i++) {
    const start = new Date().getTime();
    cp.execSync(`${TURBO_BIN} run build`, DEFAULT_EXEC_OPTS);
    const end = new Date().getTime();
    const timing = end - start;
    timings.push(timing);
  }
  return timings;
}

function saveCache() {
  // Remove any existing backup
  if (fs.existsSync(ALT_CACHE_PATH)) {
    fs.rmSync(ALT_CACHE_PATH, { recursive: true });
  }
  // copy the current cache to the backup
  if (fs.existsSync(DEFAULT_CACHE_PATH)) {
    fse.copySync(DEFAULT_CACHE_PATH, ALT_CACHE_PATH, { recursive: true });
  } else {
    // make an empty cache
    fs.mkdirSync(ALT_CACHE_PATH, { recursive: true });
  }
}

function restoreSavedCache() {
  // Remove any existing cache
  if (fs.existsSync(DEFAULT_CACHE_PATH)) {
    fs.rmSync(DEFAULT_CACHE_PATH, { recursive: true });
  }
  // Copy the backed-up cache to the real cache
  fse.copySync(ALT_CACHE_PATH, DEFAULT_CACHE_PATH, { recursive: true });
}

function cachedBuildWithDelta(): Timing[] {
  // Save existing cache just once, we'll restore from it each time
  saveCache();

  // Edit a file in place
  const file = path.join(
    REPO_PATH,
    "packages",
    "crew",
    "important-feature-0",
    "src",
    "lib",
    "important-component-0",
    "important-component-0.tsx"
  );
  const contents = fs.readFileSync(file).toString("utf-8");
  // make a small edit
  const updated = contents.replace("-0!", "-0!!");
  fs.writeFileSync(file, updated);

  const timings: Timing[] = [];
  for (let i = 0; i < REPETITIONS; i++) {
    // Make sure we're starting with the cache from before we make the source code edit
    restoreSavedCache();
    const start = new Date().getTime();
    cp.execSync(`${TURBO_BIN} run build`, DEFAULT_EXEC_OPTS);
    const end = new Date().getTime();
    const timing = end - start;
    timings.push(timing);
  }
  return timings;
}

function cachedBuildWithDependencyChange(): Timing[] {
  // Save existing cache just once, we'll restore from it each time
  saveCache();

  // Edit a dependency
  const file = path.join(REPO_PATH, "apps", "navigation", "package.json");
  const contents = JSON.parse(fs.readFileSync(file).toString("utf-8"));
  contents.dependencies["crew-important-feature-0"] = "*";
  fs.writeFileSync(file, JSON.stringify(contents, null, 2));

  const timings: Timing[] = [];
  for (let i = 0; i < REPETITIONS; i++) {
    // Make sure we're starting with the cache from before we made the dependency edit
    restoreSavedCache();
    const start = new Date().getTime();
    cp.execSync(`${TURBO_BIN} run build`, DEFAULT_EXEC_OPTS);
    const end = new Date().getTime();
    const timing = end - start;
    timings.push(timing);
  }
  return timings;
}

class Benchmarks {
  private readonly benchmarks: Benchmark[] = [];
  private readonly tbirdEvents: TBirdEvent[] = [];

  constructor(
    private readonly benchmarkFile: string,
    private readonly tinybirdFile: string,
    private readonly commitSha: string,
    private readonly commitTimestamp: Date,
    private readonly platform: string
  ) {}

  run(name: string, b: () => Timing[]) {
    console.log(name);
    const timings = b();
    const max = Math.max(...timings);
    const min = Math.min(...timings);
    const avg = timings.reduce((a, b) => a + b, 0) / timings.length;
    this.benchmarks.push({
      name,
      value: avg,
      unit: "ms",
      range: String(max - min),
    });
    timings.forEach((t) => {
      this.tbirdEvents.push({
        commitSha: this.commitSha,
        commitTimestamp: this.commitTimestamp,
        platform: this.platform,
        benchmark: name,
        durationMs: t,
      });
    });
  }

  flush() {
    console.log(JSON.stringify(this.benchmarks, null, 2));
    fs.writeFileSync(
      this.benchmarkFile,
      JSON.stringify(this.benchmarks, null, 2)
    );
    const stream = ndjson.stringify();
    const fd = fs.openSync(this.tinybirdFile, "w");
    stream.on("data", (line) => {
      fs.writeSync(fd, line);
    });
    this.tbirdEvents.forEach((t) => {
      stream.write(t);
    });
    stream.end();
    fs.closeSync(fd);
  }
}

cp.execSync(`${TURBO_BIN} --version`, { stdio: "inherit" });

function getCommitDetails(): { commitSha: string; commitTimestamp: Date } {
  const envSha = process.env["GITHUB_SHA"];
  if (envSha === undefined) {
    return {
      commitSha: "unknown sha",
      commitTimestamp: new Date(),
    };
  }
  const buf = cp.execSync(`git show -s --format=%ci ${envSha}`);
  const dateString = String(buf).trim();
  const commitTimestamp = new Date(dateString);
  return {
    commitSha: envSha,
    commitTimestamp,
  };
}

const { commitSha, commitTimestamp } = getCommitDetails();
const platform = process.env["RUNNER_OS"] ?? "unknown";

console.log("setup");
setup();
const benchmark = new Benchmarks(
  "benchmarks.json",
  "tinybird.ndjson",
  commitSha,
  commitTimestamp,
  platform
);
benchmark.run("Clean Build", cleanBuild);
benchmark.run("Cached Build - No Change", cachedBuild);
benchmark.run("Cached Build - Code Change", cachedBuildWithDelta);
benchmark.run(
  "Cached Build - Dependency Change",
  cachedBuildWithDependencyChange
);
benchmark.flush();
