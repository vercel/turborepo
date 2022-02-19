import execa from "execa";
import * as uvu from "uvu";
import * as assert from "uvu/assert";
import { Monorepo } from "../monorepo";
import path from "path";

const basicPipeline = {
  pipeline: {
    test: {
      outputs: [],
    },
    lint: {
      outputs: [],
    },
    build: {
      dependsOn: ["^build"],
      outputs: ["dist/**"],
    },
  },
  globalDependencies: ["$GLOBAL_ENV_DEPENDENCY"],
};

// This is injected by github actions
process.env.TURBO_TOKEN = "";

let suites = [];
for (let npmClient of ["yarn", "berry", "pnpm", "npm"] as const) {
  const Suite = uvu.suite(`${npmClient}`);
  const repo = new Monorepo("basics");
  repo.init(npmClient, basicPipeline);
  repo.install();
  repo.addPackage("a", ["b"]);
  repo.addPackage("b");
  repo.addPackage("c");
  repo.linkPackages();
  runSmokeTests(Suite, repo, npmClient);
  const sub = new Monorepo("in-subdirectory");
  sub.init(npmClient, basicPipeline, "js");
  sub.install();
  sub.addPackage("a", ["b"]);
  sub.addPackage("b");
  sub.addPackage("c");
  sub.linkPackages();
  runSmokeTests(Suite, sub, npmClient, {
    cwd: path.join(sub.root, sub.subdir),
  });
  suites.push(Suite);
  // test that turbo can run from a subdirectory
}

for (let s of suites) {
  s.run();
}

function runSmokeTests<T>(
  suite: uvu.Test<T>,
  repo: Monorepo,
  npmClient: "yarn" | "berry" | "pnpm" | "npm",
  options: execa.SyncOptions<string> = {}
) {
  suite.after(() => {
    repo.cleanup();
  });

  suite(
    `${npmClient} runs tests and logs${
      options.cwd ? " from " + options.cwd : ""
    }`,
    async () => {
      const results = repo.turbo("run", ["test", "--stream"], options);
      assert.equal(0, results.exitCode, "exit code should be 0");
      const commandOutput = getCommandOutputAsArray(results);
      const hash = getHashFromOutput(commandOutput, "c#test");
      assert.ok(!!hash, "No hash for c#test");
      const cachedLogFilePath = getCachedLogFilePathForTask(
        getCachedDirForHash(repo, hash),
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
    `${npmClient} handles filesystem changes${
      options.cwd ? " from " + options.cwd : ""
    }`,
    async () => {
      repo.newBranch("my-feature-branch");
      repo.commitFiles({
        [path.join("packages", "a", "test.js")]: `console.log('testingz a');`,
      });

      const sinceCommandOutputNoCache = getCommandOutputAsArray(
        repo.turbo(
          "run",
          ["test", "--since=main", "--stream", "--no-cache"],
          options
        )
      );
      assert.fixture(
        `• Packages changed since main: a`,
        sinceCommandOutputNoCache[0],
        "Calculates changed packages (--since)"
      );
      assert.fixture(
        `• Packages in scope: a`,
        sinceCommandOutputNoCache[1],
        "Packages in scope"
      );
      assert.fixture(
        `• Running test in 1 packages`,
        sinceCommandOutputNoCache[2],
        "Runs only in changed packages"
      );
      assert.fixture(
        sinceCommandOutputNoCache[3],
        `a:test: cache miss, executing ${getHashFromOutput(
          sinceCommandOutputNoCache,
          "a#test"
        )}`,
        "Cache miss in changed package"
      );

      const sinceCommandOutput = getCommandOutputAsArray(
        repo.turbo("run", ["test", "--since=main", "--stream"], options)
      );

      assert.fixture(
        `• Packages changed since main: a`,
        sinceCommandOutput[0],
        "Calculates changed packages (--since)"
      );
      assert.fixture(
        `• Packages in scope: a`,
        sinceCommandOutput[1],
        "Packages in scope"
      );
      assert.fixture(
        `• Running test in 1 packages`,
        sinceCommandOutput[2],
        "Runs only in changed packages"
      );
      assert.fixture(
        sinceCommandOutput[3],
        `a:test: cache miss, executing ${getHashFromOutput(
          sinceCommandOutput,
          "a#test"
        )}`,
        "Cache miss in changed package"
      );

      // Check cache hit after another run
      const sinceCommandSecondRunOutput = getCommandOutputAsArray(
        repo.turbo("run", ["test", "--since=main", "--stream"], options)
      );
      assert.equal(
        `• Packages changed since main: a`,
        sinceCommandSecondRunOutput[0],
        "Calculates changed packages (--since) after a second run"
      );
      assert.equal(
        `• Packages in scope: a`,
        sinceCommandSecondRunOutput[1],
        "Packages in scope after a second run"
      );
      assert.equal(
        `• Running test in 1 packages`,
        sinceCommandSecondRunOutput[2],
        "Runs only in changed packages after a second run"
      );

      assert.fixture(
        sinceCommandSecondRunOutput[3],
        `a:test: cache hit, replaying output ${getHashFromOutput(
          sinceCommandSecondRunOutput,
          "a#test"
        )}`,

        "Cache hit in changed package after a second run"
      );

      // Check that hashes are different and trigger a cascade
      repo.commitFiles({
        [path.join("packages", "b", "test.js")]: `console.log('testingz b');`,
      });

      const commandOnceBHasChangedOutput = getCommandOutputAsArray(
        repo.turbo("run", ["test", "--stream"], options)
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
          l.startsWith("c:test: cache hit, replaying output")
        ) >= 0,
        "After running, changing source of b, and running `turbo run test` again, should print `c:test: cache hit, replaying output` since c should not be impacted by changes to b"
      );

      const scopeCommandOutput = getCommandOutputAsArray(
        repo.turbo("run", ["test", '--scope="!b"', "--stream"], options)
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

  if (npmClient === "yarn") {
    // Test `turbo prune --scope=a`
    // @todo refactor with other package managers
    suite(
      `${npmClient} + turbo prune${options.cwd ? " from " + options.cwd : ""}`,
      async () => {
        const pruneCommandOutput = getCommandOutputAsArray(
          repo.turbo("prune", ["--scope=a"], options)
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
          "out/yarn.lock",
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
        const install = repo.run("install", ["--frozen-lockfile"], {
          cwd: options.cwd
            ? path.join(options.cwd, "out")
            : path.join(repo.root, "out"),
        });
        assert.is(
          install.exitCode,
          0,
          "Expected yarn install --frozen-lockfile to succeed"
        );
      }
    );
  }
}

type PackageManager = "yarn" | "pnpm" | "npm" | "berry";

// getLockfileForPackageManager returns the name of the lockfile for the given package manager
function getLockfileForPackageManager(ws: PackageManager) {
  switch (ws) {
    case "yarn":
      return "yarn.lock";
    case "pnpm":
      return "pnpm-lock.yaml";
    case "npm":
      return "package-lock.json";
    case "berry":
      return "yarn.lock";
    default:
      throw new Error(`Unknown package manager: ${ws}`);
  }
}

function getCommandOutputAsArray(
  results: execa.ExecaSyncReturnValue<string>
): string[] {
  return (results.stdout + results.stderr).split("\n");
}

function getHashFromOutput(lines: string[], taskId: string): string {
  const normalizedTaskId = taskId.replace("#", ":");
  const line = lines.find((l) => l.startsWith(normalizedTaskId));
  const splitMessage = line.split(" ");
  const hash = splitMessage[splitMessage.length - 1];
  return hash;
}

function getCachedDirForHash(repo: Monorepo, hash: string): string {
  return path.join(
    repo.subdir ? repo.subdir : ".",
    "node_modules",
    ".cache",
    "turbo",
    hash
  );
}

function getCachedLogFilePathForTask(
  cacheDir: string,
  taskName: string
): string {
  return path.join(cacheDir, ".turbo", `turbo-${taskName}.log`);
}
