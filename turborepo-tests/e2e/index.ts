import execa from "execa";
import tar from "tar";
import { Readable } from "stream";
import { ZstdCodec } from "zstd-codec";
import * as uvu from "uvu";
import * as assert from "uvu/assert";
import { Monorepo } from "./monorepo";
import path from "path";
import * as fs from "fs";

// We set this explicitly to stream, so we can lock into to streaming logs (i.e. not "auto") behavior.
//
// We do this because when these tests are invoked in CI (through .github/actions/test.yml), they will
// inherit the GITHUB_ACTIONS=true env var, and each of the `turbo run` invocations into behavior
// we do not want. Since prysk mainly tests log output, this extra behavior will break all the tests
// and can be unpredictable over time, as we make "auto" do more magic.
//
// Note: since these tests are invoked _through_ turbo, the ideal setup would be to pass --env-mode=strict
// so we can prevent the `GITHUB_ACTIONS` env var from being passed down here from the top level turbo.
// But as of now, this breaks our tests (and I'm not sure why). If we make that work, we can remove this
// explicit locking of log order. See PR attempt here: https://github.com/vercel/turbo/pull/5324
process.env.TURBO_LOG_ORDER = "stream";

const basicPipeline = {
  pipeline: {
    test: {
      dependsOn: ["^build"],
      outputs: [],
    },
    lint: {
      inputs: ["build.js", "lint.js"],
      outputs: [],
    },
    build: {
      dependsOn: ["^build"],
      outputs: ["dist/**", "!dist/cache/**"],
    },
    "//#build": {
      dependsOn: [],
      outputs: ["dist/**"],
      inputs: ["rootbuild.js"],
    },
    "//#special": {
      dependsOn: ["^build"],
      outputs: ["dist/**"],
      inputs: [],
    },
    "//#args": {
      dependsOn: [],
      outputs: [],
    },
  },
  globalEnv: ["GLOBAL_ENV_DEPENDENCY"],
};

const prunePipeline = {
  ...basicPipeline,
  pipeline: {
    ...basicPipeline.pipeline,
    // add some package specific pipeline tasks to test pruning
    "a#build": {
      outputs: ["dist/**", "!dist/cache/**"],
    },
    "c#build": {
      outputs: ["dist/**", "!dist/cache/**"],
    },
  },
};

const explicitPrunePipeline = {
  ...basicPipeline,
  pipeline: {
    ...basicPipeline.pipeline,
    // add some package specific pipeline tasks to test pruning
    "a#build": {
      dependsOn: ["b#build"],
      outputs: ["dist/**", "!dist/cache/**"],
    },
    "b#build": {
      outputs: ["dist/**", "!dist/cache/**"],
    },
    "c#build": {
      outputs: ["dist/**", "!dist/cache/**"],
    },
  },
};

const testCombinations = [
  { npmClient: "yarn" as const, pipeline: basicPipeline },
  { npmClient: "berry" as const, pipeline: basicPipeline },
  { npmClient: "pnpm6" as const, pipeline: basicPipeline },
  { npmClient: "pnpm" as const, pipeline: basicPipeline },
  { npmClient: "npm" as const, pipeline: basicPipeline },

  // there is probably no need to test every
  // pipeline against every package manager,
  // so specify directly rather than use the
  // cartesian product
  {
    npmClient: "yarn" as const,
    pipeline: prunePipeline,
    name: "basicPrune",
    excludePrune: ["c#build"],
    includePrune: ["a#build"],
  }, // expect c#build to be removed, since there is no dep between a -> c
  {
    npmClient: "yarn" as const,
    pipeline: explicitPrunePipeline,
    name: "explicitDepPrune",
    excludePrune: ["c#build"],
    includePrune: ["a#build", "b#build"],
  }, // expect c#build to be included, since a depends on c
];

// This is injected by github actions
process.env.TURBO_TOKEN = "";

let suites: uvu.uvu.Test<uvu.Context>[] = [];
for (let {
  npmClient,
  pipeline,
  name,
  includePrune,
  excludePrune,
} of testCombinations) {
  const Suite = uvu.suite(`${name ?? npmClient}`);

  const repo = new Monorepo("basics");
  repo.init(npmClient, pipeline);
  repo.install();
  repo.addPackage("a", ["b"]);
  repo.addPackage("b");
  repo.addPackage("c");
  repo.linkPackages();
  repo.expectCleanGitStatus();
  runSmokeTests(Suite, repo, npmClient, includePrune ?? [], excludePrune ?? []);

  const sub = new Monorepo("in-subdirectory");
  sub.init(npmClient, pipeline, "js");
  sub.install();
  sub.addPackage("a", ["b"]);
  sub.addPackage("b");
  sub.addPackage("c");
  sub.linkPackages();

  runSmokeTests(Suite, sub, npmClient, includePrune ?? [], excludePrune ?? [], {
    cwd: sub.subdir ? path.join(sub.root, sub.subdir) : sub.root,
  });

  suites.push(Suite);
  // test that turbo can run from a subdirectory
}

for (let suite of suites) {
  suite.run();
}

type Task = {
  taskId: string;
  task: string;
  package: string;
  hash: string;
  command: string;
  outputs: string[];
  logFile: string;
  directory: string;
  dependencies: string[];
  dependents: string[];
};

type DryRun = {
  packages: string[];
  tasks: Task[];
};

const matchTask =
  <T, V>(predicate: (dryRun: DryRun, val: T) => V) =>
  (dryRun: DryRun) =>
  (val: T): V => {
    return predicate(dryRun, val);
  };
const includesTaskIdPredicate = (dryRun: DryRun, task: string): boolean => {
  for (const entry of dryRun.tasks) {
    if (entry.taskId === task) {
      return true;
    }
  }
  return false;
};
const includesTaskId = matchTask(includesTaskIdPredicate);
const taskHashPredicate = (dryRun: DryRun, taskId: string): string => {
  for (const entry of dryRun.tasks) {
    if (entry.taskId === taskId) {
      return entry.hash;
    }
  }
  throw new Error(`missing task with id ${taskId}`);
};

function runSmokeTests<T>(
  suite: uvu.Test<T>,
  repo: Monorepo,
  npmClient: "yarn" | "berry" | "pnpm6" | "pnpm" | "npm",
  includePrune: string[],
  excludePrune: string[],
  options: execa.SyncOptions<string> = {}
) {
  suite.after(() => {
    repo.cleanup();
  });

  suite(
    `${npmClient} builds${options.cwd ? " from " + options.cwd : ""}`,
    async () => {
      const results = repo.turbo("run", ["build", "--dry=json"], options);
      const dryRun: DryRun = JSON.parse(results.stdout);
      // expect to run all packages
      const expectTaskId = includesTaskId(dryRun);
      for (const pkg of ["a", "b", "c", "//"]) {
        assert.ok(
          dryRun.packages.includes(pkg),
          `Expected to include package ${pkg}`
        );
        assert.ok(
          expectTaskId(pkg + "#build"),
          `Expected to include task ${pkg}#build`
        );
      }

      // actually run the build
      const buildOutput = getCommandOutputAsArray(
        repo.turbo("run", ["build"], options)
      );
      assert.ok(
        buildOutput.includes("//:build: building"),
        "Missing root build"
      );

      // assert that hashes are stable across multiple runs
      const secondRun = repo.turbo("run", ["build", "--dry=json"], options);
      const secondDryRun: DryRun = JSON.parse(secondRun.stdout);

      repo.turbo("run", ["build"], options);

      const thirdRun = repo.turbo("run", ["build", "--dry=json"], options);
      const thirdDryRun: DryRun = JSON.parse(thirdRun.stdout);
      const getThirdRunHash = matchTask(taskHashPredicate)(thirdDryRun);
      for (const entry of secondDryRun.tasks) {
        assert.equal(
          getThirdRunHash(entry.taskId),
          entry.hash,
          `Hashes for ${entry.taskId} did not match`
        );
      }
    }
  );

  suite(
    `${npmClient} runs tests and logs${
      options.cwd ? " from " + options.cwd : ""
    }`,
    async () => {
      const results = repo.turbo("run", ["test"], options);
      assert.equal(0, results.exitCode, "exit code should be 0");
      const commandOutput = getCommandOutputAsArray(results);
      const hash = getHashFromOutput(commandOutput, "c#test");
      assert.ok(!!hash, "No hash for c#test");

      const cacheItemPath = getCacheItemForHash(repo, hash);
      await extractZst(path.join(repo.root, cacheItemPath), repo.root);

      const cachedLogFilePath = getCachedLogFilePathForTask(
        repo,
        path.join("packages", "c"),
        "test"
      );
      let text = "";
      assert.not.throws(() => {
        text = repo.readFileSync(cachedLogFilePath);
      }, `Could not read cached log file from cache ${cachedLogFilePath}`);
      assert.ok(text.includes("testing c"), "Contains correct output");
    }
  );

  suite(
    `${npmClient} runs lint and logs${
      options.cwd ? " from " + options.cwd : ""
    }`,
    async () => {
      const results = repo.turbo("run", ["lint"], options);
      assert.equal(0, results.exitCode, "exit code should be 0");
      const commandOutput = getCommandOutputAsArray(results);
      const hash = getHashFromOutput(commandOutput, "c#lint");
      assert.ok(!!hash, "No hash for c#lint");

      const cacheItemPath = getCacheItemForHash(repo, hash);
      await extractZst(path.join(repo.root, cacheItemPath), repo.root);

      const cachedLogFilePath = getCachedLogFilePathForTask(
        repo,
        path.join("packages", "c"),
        "lint"
      );
      let text = "";
      assert.not.throws(() => {
        text = repo.readFileSync(cachedLogFilePath);
      }, `Could not read cached log file from cache ${cachedLogFilePath}`);
      assert.ok(text.includes("linting c"), "Contains correct output");
    }
  );

  suite(
    `${npmClient} handles filesystem changes${
      options.cwd ? " from " + options.cwd : ""
    }`,
    async () => {
      repo.newBranch("my-feature-branch");
      repo.commitFiles({
        [path.join("packages", "a", "test.js")]: `console.log('testingz a');`,
      });
      const sinceCommandOutputNoCache = getCommandOutputAsArray(
        repo.turbo("run", ["test", "--since=main", "--no-cache"], options)
      );

      assert.fixture(
        sinceCommandOutputNoCache[0],
        `• Packages in scope: a`,
        "Packages in scope"
      );
      assert.fixture(
        sinceCommandOutputNoCache[1],
        `• Running test in 1 packages`,
        "Runs only in changed packages"
      );

      assert.ok(
        sinceCommandOutputNoCache.includes(
          `a:test: cache miss, executing ${getHashFromOutput(
            sinceCommandOutputNoCache,
            "a#test"
          )}`
        ),
        "Cache miss in changed package"
      );

      const sinceCommandOutput = getCommandOutputAsArray(
        repo.turbo(
          "run",
          ["test", "--since=main", "--output-logs=hash-only"],
          options
        )
      );

      assert.fixture(
        sinceCommandOutput[0],
        `• Packages in scope: a`,
        "Packages in scope"
      );
      assert.fixture(
        sinceCommandOutput[1],
        `• Running test in 1 packages`,
        "Runs only in changed packages"
      );

      assert.ok(
        sinceCommandOutput.includes(
          `a:test: cache miss, executing ${getHashFromOutput(
            sinceCommandOutput,
            "a#test"
          )}`
        ),
        "Cache miss in changed package"
      );

      // Check cache hit after another run
      const sinceCommandSecondRunOutput = getCommandOutputAsArray(
        repo.turbo(
          "run",
          ["test", "--since=main", "--output-logs=hash-only"],
          options
        )
      );
      assert.equal(
        sinceCommandSecondRunOutput[0],
        `• Packages in scope: a`,
        "Packages in scope after a second run"
      );
      assert.equal(
        sinceCommandSecondRunOutput[1],
        `• Running test in 1 packages`,
        "Runs only in changed packages after a second run"
      );

      assert.ok(
        sinceCommandSecondRunOutput.includes(
          `b:build: cache hit, suppressing logs ${getHashFromOutput(
            sinceCommandSecondRunOutput,
            "b#build"
          )}`
        ),
        "Cache hit building dependency after a second run"
      );

      assert.ok(
        sinceCommandSecondRunOutput.includes(
          `a:test: cache hit, suppressing logs ${getHashFromOutput(
            sinceCommandSecondRunOutput,
            "a#test"
          )}`
        ),
        "Cache hit in changed package after a second run"
      );

      // run a task without dependencies

      // ensure that uncommitted irrelevant changes are also ignored
      repo.modifyFiles({
        [path.join("packages", "a", "README.md")]: "important text",
      });
      const lintOutput = getCommandOutputAsArray(
        repo.turbo(
          "run",
          ["lint", "--filter=a", "--output-logs=hash-only"],
          options
        )
      );
      assert.equal(
        lintOutput[0],
        `• Packages in scope: a`,
        "Packages in scope for lint"
      );
      assert.ok(
        lintOutput.includes(
          `a:lint: cache hit, suppressing logs ${getHashFromOutput(
            lintOutput,
            "a#lint"
          )}`
        ),
        "Cache hit, a has changed but not a file lint depends on"
      );

      // Check that hashes are different and trigger a cascade
      repo.commitFiles({
        [path.join("packages", "b", "test.js")]: `console.log('testingz b');`,
      });

      const secondLintRun = getCommandOutputAsArray(
        repo.turbo(
          "run",
          ["lint", "--filter=a", "--output-logs=hash-only"],
          options
        )
      );

      assert.equal(
        secondLintRun[0],
        `• Packages in scope: a`,
        "Packages in scope for lint"
      );
      assert.ok(
        secondLintRun.includes(
          `a:lint: cache hit, suppressing logs ${getHashFromOutput(
            secondLintRun,
            "a#lint"
          )}`
        ),
        "Cache hit, dependency changes are irrelevant for lint task"
      );

      repo.commitFiles({
        [path.join("packages", "a", "lint.js")]: "console.log('lintingz a')",
      });

      const thirdLintRun = getCommandOutputAsArray(
        repo.turbo(
          "run",
          ["lint", "--filter=a", "--output-logs=hash-only"],
          options
        )
      );

      assert.equal(
        thirdLintRun[0],
        `• Packages in scope: a`,
        "Packages in scope for lint"
      );
      assert.ok(
        thirdLintRun.includes(
          `a:lint: cache miss, executing ${getHashFromOutput(
            thirdLintRun,
            "a#lint"
          )}`
        ),
        "Cache miss, we changed a file that lint uses as an input"
      );

      const commandOnceBHasChangedOutput = getCommandOutputAsArray(
        repo.turbo("run", ["test"], options)
      );

      assert.fixture(
        `• Packages in scope: a, b, c`,
        commandOnceBHasChangedOutput[0],
        "After running, changing source of b, and running `turbo run test` again, should print `Packages in scope: a, b, c`"
      );
      assert.fixture(
        `• Running test in 3 packages`,
        commandOnceBHasChangedOutput[1],
        "After running, changing source of b, and running `turbo run test` again, should print `Running in 3 packages`"
      );
      assert.ok(
        commandOnceBHasChangedOutput.findIndex((l) =>
          l.startsWith("a:test: cache miss, executing")
        ) >= 0,
        "After running, changing source of b, and running `turbo run test` again, should print `a:test: cache miss, executing` since a depends on b and b has changed"
      );
      assert.ok(
        commandOnceBHasChangedOutput.findIndex((l) =>
          l.startsWith("b:test: cache miss, executing")
        ) >= 0,
        "After running, changing source of b, and running `turbo run test` again, should print `b:test: cache miss, executing` since b has changed"
      );
      assert.ok(
        commandOnceBHasChangedOutput.findIndex((l) =>
          l.startsWith("c:test: cache hit, replaying logs")
        ) >= 0,
        "After running, changing source of b, and running `turbo run test` again, should print `c:test: cache hit, replaying logs` since c should not be impacted by changes to b"
      );

      const scopeCommandOutput = getCommandOutputAsArray(
        repo.turbo("run", ["test", '--scope="!b"'], options)
      );

      assert.fixture(
        `• Packages in scope: a, c`,
        scopeCommandOutput[0],
        "Packages in scope"
      );
      assert.fixture(
        `• Running test in 2 packages`,
        scopeCommandOutput[1],
        "Runs only in changed packages"
      );
    }
  );

  suite(
    `${npmClient} runs root tasks${options.cwd ? " from " + options.cwd : ""}`,
    async () => {
      const result = getCommandOutputAsArray(
        repo.turbo("run", ["special"], options)
      );
      assert.ok(result.includes("//:special: root task"));
      const secondPass = getCommandOutputAsArray(
        repo.turbo(
          "run",
          ["special", "--filter=//", "--output-logs=hash-only"],
          options
        )
      );
      assert.ok(
        secondPass.includes(
          `//:special: cache hit, suppressing logs ${getHashFromOutput(
            secondPass,
            "//#special"
          )}`
        ),
        "Rerun of //:special should be cached"
      );
    }
  );

  suite(
    `${npmClient} passes through correct args ${
      options.cwd ? " from " + options.cwd : ""
    }`,
    async () => {
      const expectArgsPassed = (inputArgs: string[], passedArgs: string[]) => {
        const result = getCommandOutputAsArray(
          repo.turbo("run", inputArgs, options)
        );
        // Find the output logs of the test script
        const needle = "//:args: Output:";
        const script_output = result.find((line) => line.startsWith(needle));

        assert.ok(
          script_output != undefined && script_output.startsWith(needle),
          `Unable to find '//:arg' output in '${result}'`
        );
        const [node, ...args] = JSON.parse(
          script_output.substring(needle.length)
        );

        assert.match(
          node,
          "node",
          `Expected node binary path (${node}) to contain 'node'`
        );
        assert.equal(args, passedArgs);
      };

      const tests = [
        [["args", "--filter=//", "--", "--script-arg=42"], ["--script-arg=42"]],
        [["args", "--filter=//", "--", "--filter=//"], ["--filter=//"]],
        [["--filter=//", "args", "--", "--filter=//"], ["--filter=//"]],
        [
          ["args", "--", "--script-arg", "42"],
          ["--script-arg", "42"],
        ],
        [["args"], []],
        [["args", "--"], []],
        [
          ["args", "--", "--", "--"],
          ["--", "--"],
        ],
        [
          ["args", "--", "first", "--", "second"],
          ["first", "--", "second"],
        ],
        [
          ["args", "--", "-f", "--f", "---f", "----f"],
          ["-f", "--f", "---f", "----f"],
        ],
      ];

      for (const [input, expected] of tests) {
        expectArgsPassed(input, expected);
      }
    }
  );

  if (["npm", "yarn", "pnpm6", "pnpm", "berry"].includes(npmClient)) {
    // Test `turbo prune --scope=a`
    // @todo refactor with other package managers
    const [installCmd, ...installArgs] =
      getImmutableInstallForPackageManager(npmClient);
    suite(
      `${npmClient} + turbo prune${options.cwd ? " from " + options.cwd : ""}`,
      async () => {
        const scope = "a";
        const pruneCommandOutput = getCommandOutputAsArray(
          repo.turbo("prune", [`--scope=${scope}`], options)
        );
        assert.fixture(pruneCommandOutput[1], " - Added a");
        assert.fixture(pruneCommandOutput[2], " - Added b");

        let files = [];
        assert.not.throws(() => {
          files = repo.globbySync("out/**/*", {
            cwd: options.cwd ?? repo.root,
          });
        }, `Could not read generated \`out\` directory after \`turbo prune\``);
        const expected = [
          "out/package.json",
          "out/turbo.json",
          `out/${getLockfileForPackageManager(npmClient)}`,
          "out/packages/a/build.js",
          "out/packages/a/lint.js",
          "out/packages/a/package.json",
          "out/packages/a/test.js",
          "out/packages/b/build.js",
          "out/packages/b/lint.js",
          "out/packages/b/package.json",
          "out/packages/b/test.js",
        ];
        for (const file of expected) {
          assert.ok(
            files.includes(file),
            `Expected file ${file} to be generated`
          );
        }

        // grab the first turbo.json in an out folder
        let turbos = repo
          .globbySync("**/out/turbo.json")
          .map((t: string) => JSON.parse(repo.readFileSync(t)));
        for (const turbo of turbos) {
          const pipelines = Object.keys(turbo.pipeline);
          const missingInclude = includePrune.filter(
            (i) => !pipelines.includes(i)
          );
          const presentExclude = excludePrune.filter((i) =>
            pipelines.includes(i)
          );

          if (missingInclude.length || presentExclude.length) {
            assert.unreachable(
              "failed to validate prune in pipeline" +
                (missingInclude.length ? `, expecting ${missingInclude}` : "") +
                (presentExclude.length
                  ? `, not expecting ${presentExclude}`
                  : "")
            );
          }
        }

        const install = repo.run(installCmd, installArgs, {
          cwd: options.cwd
            ? path.join(options.cwd, "out")
            : path.join(repo.root, "out"),
        });
        assert.is(
          install.exitCode,
          0,
          `Expected ${npmClient} install --frozen-lockfile to succeed`
        );
      }
    );

    suite(
      `${npmClient} + turbo prune --docker${
        options.cwd ? " from " + options.cwd : ""
      }`,
      async () => {
        const scope = "a";
        const pruneCommandOutput = getCommandOutputAsArray(
          repo.turbo("prune", [`--scope=${scope}`, "--docker"], options)
        );
        assert.fixture(pruneCommandOutput[1], " - Added a");
        assert.fixture(pruneCommandOutput[2], " - Added b");

        let files: string[] = [];
        assert.not.throws(() => {
          files = repo.globbySync("out/**/*", {
            cwd: options.cwd ?? repo.root,
          });
        }, `Could not read generated \`out\` directory after \`turbo prune\``);
        const expected = [
          "out/full/package.json",
          "out/json/package.json",
          "out/full/turbo.json",
          `out/${getLockfileForPackageManager(npmClient)}`,
          "out/full/packages/a/build.js",
          "out/full/packages/a/lint.js",
          "out/full/packages/a/package.json",
          "out/json/packages/a/package.json",
          "out/full/packages/a/test.js",
          "out/full/packages/b/build.js",
          "out/full/packages/b/lint.js",
          "out/full/packages/b/package.json",
          "out/json/packages/b/package.json",
          "out/full/packages/b/test.js",
        ];
        for (const file of expected) {
          assert.ok(
            files.includes(file),
            `Expected file ${file} to be generated`
          );
        }

        // grab the first turbo.json in an out folder
        let turbos = repo
          .globbySync("**/out/turbo.json")
          .map((t: string) => JSON.parse(repo.readFileSync(t)));
        for (const turbo of turbos) {
          const pipelines = Object.keys(turbo.pipeline);
          const missingInclude = includePrune.filter(
            (i) => !pipelines.includes(i)
          );
          const presentExclude = excludePrune.filter((i) =>
            pipelines.includes(i)
          );

          if (missingInclude.length || presentExclude.length) {
            assert.unreachable(
              "failed to validate prune in pipeline" +
                (missingInclude.length ? `, expecting ${missingInclude}` : "") +
                (presentExclude.length
                  ? `, not expecting ${presentExclude}`
                  : "")
            );
          }
        }

        const install = repo.run(installCmd, installArgs, {
          cwd: options.cwd
            ? path.join(options.cwd, "out")
            : path.join(repo.root, "out"),
        });
        assert.is(
          install.exitCode,
          0,
          `Expected ${npmClient} install --frozen-lockfile to succeed`
        );
      }
    );
  }
}

type PackageManager = "yarn" | "pnpm6" | "pnpm" | "npm" | "berry";

// getLockfileForPackageManager returns the name of the lockfile for the given package manager
function getLockfileForPackageManager(ws: PackageManager) {
  switch (ws) {
    case "yarn":
      return "yarn.lock";
    case "pnpm":
      return "pnpm-lock.yaml";
    case "pnpm6":
      return "pnpm-lock.yaml";
    case "npm":
      return "package-lock.json";
    case "berry":
      return "yarn.lock";
    default:
      throw new Error(`Unknown package manager: ${ws}`);
  }
}

function getImmutableInstallForPackageManager(ws: PackageManager): string[] {
  switch (ws) {
    case "yarn":
      return ["install", "--frozen-lockfile"];
    case "pnpm":
      return ["install", "--frozen-lockfile"];
    case "pnpm6":
      return ["install", "--frozen-lockfile"];
    case "npm":
      return ["ci"];
    case "berry":
      return ["install", "--immutable"];
    default:
      throw new Error(`Unknown package manager: ${ws}`);
  }
}
function getCommandOutputAsArray(
  results: execa.ExecaSyncReturnValue<string>
): string[] {
  return (results.stdout + "\n" + results.stderr)
    .split("\n")
    .map((line) => line.replace("\r", ""));
}

function getHashFromOutput(lines: string[], taskId: string): string {
  const normalizedTaskId = taskId.replace("#", ":");
  const line = lines.find((l) => l.startsWith(normalizedTaskId));
  const splitMessage = line.split(" ");
  const hash = splitMessage[splitMessage.length - 1];
  return hash;
}

function getCacheItemForHash(repo: Monorepo, hash: string): string {
  return path.join(
    repo.subdir ? repo.subdir : ".",
    "node_modules",
    ".cache",
    "turbo",
    `${hash}.tar.zst`
  );
}

function getCachedLogFilePathForTask(
  repo: Monorepo,
  pathToPackage: string,
  taskName: string
): string {
  return path.join(
    repo.subdir ? repo.subdir : "",
    pathToPackage,
    ".turbo",
    `turbo-${taskName}.log`
  );
}

function createDecoder() {
  return new Promise((resolve) => {
    ZstdCodec.run((zstd) => resolve(new zstd.Streaming()));
  });
}

async function extractZst(zst, dest) {
  let decoder = await createDecoder();
  const fileBuffer = fs.readFileSync(zst);
  const data = new Uint8Array(
    fileBuffer.buffer.slice(
      fileBuffer.byteOffset,
      fileBuffer.byteOffset + fileBuffer.byteLength
    )
  );
  const decompressed = decoder.decompress(data);
  const stream = Readable.from(Buffer.from(decompressed));
  const output = stream.pipe(
    tar.x({
      cwd: dest,
    })
  );

  return new Promise((resolve) => {
    output.on("finish", resolve);
  });
}
