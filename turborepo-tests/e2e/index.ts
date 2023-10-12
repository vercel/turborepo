import execa from "execa";
import * as uvu from "uvu";
import * as assert from "uvu/assert";
import { Monorepo } from "./monorepo";
import path from "path";
import {
  basicPipeline,
  prunePipeline,
  explicitPrunePipeline,
} from "./fixtures";
import type { DryRun, PackageManager } from "./types";
import {
  matchTask,
  includesTaskId,
  taskHashPredicate,
  getLockfileForPackageManager,
  getImmutableInstallForPackageManager,
  getCommandOutputAsArray,
  getHashFromOutput,
  getCacheItemForHash,
  getCachedLogFilePathForTask,
  extractZst,
} from "./helpers";

const testCombinations = [
  { npmClient: "yarn" as PackageManager, pipeline: basicPipeline },
  { npmClient: "berry" as PackageManager, pipeline: basicPipeline },
  { npmClient: "pnpm6" as PackageManager, pipeline: basicPipeline },
  { npmClient: "pnpm" as PackageManager, pipeline: basicPipeline },
  { npmClient: "npm" as PackageManager, pipeline: basicPipeline },

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
for (const combo of testCombinations) {
  const {
    npmClient,
    pipeline,
    name,
    includePrune = [],
    excludePrune = [],
  } = combo;

  const Suite = uvu.suite(`${name ?? npmClient}`);

  const repo = new Monorepo({
    root: "basics",
    pm: npmClient,
    pipeline,
  });
  repo.init();
  repo.install();
  repo.addPackage("a", ["b"]);
  repo.addPackage("b");
  repo.addPackage("c");
  repo.linkPackages();
  repo.expectCleanGitStatus();
  runSmokeTests(Suite, repo, npmClient, includePrune, excludePrune);

  const sub = new Monorepo({
    root: "in-subdirectory",
    pm: npmClient,
    pipeline,
    subdir: "js",
  });
  sub.init();
  sub.install();
  sub.addPackage("a", ["b"]);
  sub.addPackage("b");
  sub.addPackage("c");
  sub.linkPackages();

  runSmokeTests(Suite, sub, npmClient, includePrune, excludePrune, {
    cwd: sub.subdir ? path.join(sub.root, sub.subdir) : sub.root,
  });

  suites.push(Suite);
  // test that turbo can run from a subdirectory
}

for (let suite of suites) {
  suite.run();
}

function runSmokeTests<T>(
  suite: uvu.Test<T>,
  repo: Monorepo,
  npmClient: PackageManager,
  includePrune: string[],
  excludePrune: string[],
  options: execa.SyncOptions<string> = {}
) {
  suite.after(() => {
    repo.cleanup();
  });

  const suffix = `${options.cwd ? " from " + options.cwd : ""}`;

  suite(`${npmClient} builds${suffix}`, async () => {
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
    assert.ok(buildOutput.includes("//:build: building"), "Missing root build");

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
  });

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
      assert.ok(!!hash, `No hash for c#lint in ${commandOutput.join("\n")}`);

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

  suite(`${npmClient} runs root tasks${suffix}`, async () => {
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
  });

  suite(`${npmClient} passes through correct args ${suffix}`, async () => {
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
  });

  // Test `turbo prune a`
  // @todo refactor with other package managers
  const [installCmd, ...installArgs] =
    getImmutableInstallForPackageManager(npmClient);

  suite(`${npmClient} + turbo prune${suffix}`, async () => {
    const scope = "a";
    const pruneCommandOutput = getCommandOutputAsArray(
      repo.turbo("prune", [scope], options)
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
      assert.ok(files.includes(file), `Expected file ${file} to be generated`);
    }

    // grab the first turbo.json in an out folder
    let turbos = repo
      .globbySync("**/out/turbo.json")
      .map((t: string) => JSON.parse(repo.readFileSync(t)));
    for (const turbo of turbos) {
      const pipelines = Object.keys(turbo.pipeline);
      const missingInclude = includePrune.filter((i) => !pipelines.includes(i));
      const presentExclude = excludePrune.filter((i) => pipelines.includes(i));

      if (missingInclude.length || presentExclude.length) {
        assert.unreachable(
          "failed to validate prune in pipeline" +
            (missingInclude.length ? `, expecting ${missingInclude}` : "") +
            (presentExclude.length ? `, not expecting ${presentExclude}` : "")
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
  });

  suite(`${npmClient} + turbo prune --docker${suffix}`, async () => {
    const scope = "a";
    const pruneCommandOutput = getCommandOutputAsArray(
      repo.turbo("prune", [scope, "--docker"], options)
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
      assert.ok(files.includes(file), `Expected file ${file} to be generated`);
    }

    // grab the first turbo.json in an out folder
    let turbos = repo
      .globbySync("**/out/turbo.json")
      .map((t: string) => JSON.parse(repo.readFileSync(t)));
    for (const turbo of turbos) {
      const pipelines = Object.keys(turbo.pipeline);
      const missingInclude = includePrune.filter((i) => !pipelines.includes(i));
      const presentExclude = excludePrune.filter((i) => pipelines.includes(i));

      if (missingInclude.length || presentExclude.length) {
        assert.unreachable(
          "failed to validate prune in pipeline" +
            (missingInclude.length ? `, expecting ${missingInclude}` : "") +
            (presentExclude.length ? `, not expecting ${presentExclude}` : "")
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
  });
}
