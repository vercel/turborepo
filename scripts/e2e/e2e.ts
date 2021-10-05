import execa from "execa";
import { test } from "uvu";
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
};

for (let npmClient of ["yarn", "pnpm", "npm"] as const) {
  const repo = new Monorepo("basics");
  repo.init(npmClient, basicPipeline);
  repo.install();
  repo.addPackage("a", ["b"]);
  repo.addPackage("b");
  repo.addPackage("c");
  repo.linkPackages();
  runSmokeTests(repo, npmClient);
  const sub = new Monorepo("in-subdirectory");
  sub.init(npmClient, basicPipeline, "js");
  sub.install();
  sub.addPackage("a", ["b"]);
  sub.addPackage("b");
  sub.addPackage("c");
  sub.linkPackages();
  runSmokeTests(sub, npmClient, {
    cwd: path.join(sub.root, sub.subdir),
  });
  // test that turbo can run from a subdirectory
}

test.run();

function runSmokeTests(
  repo: Monorepo,
  npmClient: "yarn" | "pnpm" | "npm",
  options: execa.SyncOptions<string> = {}
) {
  test(`${npmClient} runs tests and logs ${
    options.cwd ? " from " + options.cwd : ""
  } `, async () => {
    const results = repo.turbo("run", ["test", "--stream", "-vvv"], options);
    const out = results.stdout + results.stderr;
    assert.equal(0, results.exitCode);
    const lines = out.split("\n");
    const chash = lines.find((l) => l.startsWith("c:test"));
    assert.ok(!!chash, "No hash for c:test");
    const splitMessage = chash.split(" ");
    const hash = splitMessage[splitMessage.length - 1];
    const logFilePath = `${
      repo.subdir ? repo.subdir + "/" : ""
    }node_modules/.cache/turbo/${hash}/.turbo/turbo-test.log`;
    let text = "";
    assert.not.throws(() => {
      text = repo.readFileSync(logFilePath);
    }, `Could not read log file from cache ${logFilePath}`);

    assert.ok(text.includes("testing c"), "Contains correct output");
    repo.newBranch("my-feature-branch");
    repo.commitFiles({
      [`packages/a/test.js`]: `console.log('testingz a');`,
    });

    const sinceResults = repo.turbo(
      "run",
      ["test", "--since=main", "--stream", "-vvv"],
      options
    );
    const testCLine = (sinceResults.stdout + sinceResults.stderr).split("\n");
    assert.equal(
      `• Packages changed since main: a`,
      testCLine[0],
      "Calculates changed packages (--since)"
    );
    assert.equal(
      `• Running test in 1 packages`,
      testCLine[1],
      "Runs only in changed packages"
    );
    assert.ok(
      testCLine[2].startsWith(`a:test: cache miss, executing`),
      "Cache miss in changed package"
    );

    repo.cleanup();
  });
}
