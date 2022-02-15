import cp from "child_process";
import fs from "fs";
import fse from "fs-extra";
import path from "path";

const REPO_ROOT = "large-monorepo";
const REPO_ORIGIN = "https://github.com/gsoltis/large-monorepo.git";
const REPO_PATH = path.join(process.cwd(), REPO_ROOT);
const REPETITIONS = 5;

const DEFAULT_EXEC_OPTS = { stdio: "ignore" as const, cwd: REPO_PATH };
const TURBO_BIN = path.resolve(path.join("..", "cli", "turbo"));
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

type Benchmark = {
  name: string;
  unit: string;
  value: number;
  range?: string;
  extra?: string;
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

function cleanBuild(): Benchmark {
  const timings: number[] = [];
  let total = 0;
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
    total += timing;
    timings.push(timing);
  }
  const avg = total / REPETITIONS;
  const max = Math.max(...timings);
  const min = Math.min(...timings);
  return {
    name: "Clean Build",
    value: avg,
    unit: "ms",
    range: String(max - min),
  };
}

function cachedBuild(): Benchmark {
  const timings: number[] = [];
  let total = 0;
  for (let i = 0; i < REPETITIONS; i++) {
    const start = new Date().getTime();
    cp.execSync(`${TURBO_BIN} run build`, DEFAULT_EXEC_OPTS);
    const end = new Date().getTime();
    const timing = end - start;
    total += timing;
    timings.push(timing);
  }
  const avg = total / REPETITIONS;
  const max = Math.max(...timings);
  const min = Math.min(...timings);
  return {
    name: "Cached Build - no changes",
    value: avg,
    unit: "ms",
    range: String(max - min),
  };
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

function cachedBuildWithDelta(): Benchmark {
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

  const timings: number[] = [];
  let total = 0;
  for (let i = 0; i < REPETITIONS; i++) {
    // Make sure we're starting with the cache from before we make the source code edit
    restoreSavedCache();
    const start = new Date().getTime();
    cp.execSync(`${TURBO_BIN} run build`, DEFAULT_EXEC_OPTS);
    const end = new Date().getTime();
    const timing = end - start;
    total += timing;
    timings.push(timing);
  }
  const avg = total / REPETITIONS;
  const max = Math.max(...timings);
  const min = Math.min(...timings);
  return {
    name: "Cached Build - source code change",
    value: avg,
    unit: "ms",
    range: String(max - min),
  };
}

function cachedBuildWithDependencyChange(): Benchmark {
  // Save existing cache just once, we'll restore from it each time
  saveCache();

  // Edit a dependency
  const file = path.join(REPO_PATH, "apps", "navigation", "package.json");
  const contents = JSON.parse(fs.readFileSync(file).toString("utf-8"));
  contents.dependencies["crew-important-feature-0"] = "*";
  fs.writeFileSync(file, JSON.stringify(contents, null, 2));

  const timings: number[] = [];
  let total = 0;
  for (let i = 0; i < REPETITIONS; i++) {
    // Make sure we're starting with the cache from before we made the dependency edit
    restoreSavedCache();
    const start = new Date().getTime();
    cp.execSync(`${TURBO_BIN} run build`, DEFAULT_EXEC_OPTS);
    const end = new Date().getTime();
    const timing = end - start;
    total += timing;
    timings.push(timing);
  }
  const avg = total / REPETITIONS;
  const max = Math.max(...timings);
  const min = Math.min(...timings);
  return {
    name: "Cached Build - dependency change",
    value: avg,
    unit: "ms",
    range: String(max - min),
  };
}

cp.execSync(`${TURBO_BIN} --version`, { stdio: "inherit" });

const benchmarks: Benchmark[] = [];
console.log("setup");
setup();
console.log("clean build");
benchmarks.push(cleanBuild());
console.log("cached build - no change");
benchmarks.push(cachedBuild());
console.log("cached build - code change");
benchmarks.push(cachedBuildWithDelta());
console.log("cached build - dependency change");
benchmarks.push(cachedBuildWithDependencyChange());
console.log(JSON.stringify(benchmarks, null, 2));
fs.writeFileSync("benchmarks.json", JSON.stringify(benchmarks, null, 2));
