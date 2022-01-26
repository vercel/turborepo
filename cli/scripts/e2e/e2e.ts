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
    `${npmClient} runs tests and logs ${
      options.cwd ? " from " + options.cwd : ""
    } `,
    async () => {
      const results = repo.turbo("run", ["test", "--stream", "-vvv"], options);
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
    `${npmClient} handles filesystem changes ${
      options.cwd ? " from " + options.cwd : ""
    } `,
    async () => {
      repo.newBranch("my-feature-branch");
      repo.commitFiles({
        [path.join("packages", "a", "test.js")]: `console.log('testingz a');`,
      });

      const sinceCommandOutput = getCommandOutputAsArray(
        repo.turbo("run", ["test", "--since=main", "--stream", "-vvv"], options)
      );

      assert.equal(
        `• Packages changed since main: a`,
        sinceCommandOutput[0],
        "Calculates changed packages (--since)"
      );
      assert.equal(
        `• Packages in scope: a`,
        sinceCommandOutput[1],
        "Packages in scope"
      );
      assert.equal(
        `• Running test in 1 packages`,
        sinceCommandOutput[2],
        "Runs only in changed packages"
      );
      assert.ok(
        sinceCommandOutput[3].startsWith(`a:test: cache miss, executing`),
        "Cache miss in changed package"
      );

      // Check cache hit after another run
      const sinceCommandSecondRunOutput = getCommandOutputAsArray(
        repo.turbo("run", ["test", "--since=main", "--stream", "-vvv"], options)
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

      assert.ok(
        sinceCommandSecondRunOutput[3].startsWith(
          `a:test: cache hit, replaying output`
        ),
        "Cache hit in changed package after a second run"
      );

      // Check that hashes are different and trigger a cascade
      repo.commitFiles({
        [path.join("packages", "b", "test.js")]: `console.log('testingz b');`,
      });

      const commandOnceBHasChangedOutput = getCommandOutputAsArray(
        repo.turbo("run", ["test", "--stream"], options)
      );

      assert.equal(
        `• Packages in scope: a, b, c`,
        commandOnceBHasChangedOutput[0],
        "After running, changing source of b, and running `turbo run test` again, should print `Packages in scope: a, b, c`"
      );
      assert.equal(
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
    }
  );
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
    repo.subdir ? repo.subdir + "/" : "",
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
