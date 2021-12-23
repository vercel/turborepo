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

// This is injected by github actions
process.env.TURBO_TOKEN = "";

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
    const logFilePath = path.join(
      repo.subdir ? repo.subdir + "/" : "",
      "node_modules",
      ".cache",
      "turbo",
      hash,
      ".turbo",
      "turbo-test.log"
    );
    let text = "";
    assert.not.throws(() => {
      text = repo.readFileSync(logFilePath);
    }, `Could not read log file from cache ${logFilePath}`);

    assert.ok(text.includes("testing c"), "Contains correct output");

    repo.newBranch("my-feature-branch");
    repo.commitFiles({
      [path.join("packages", "a", "test.js")]: `console.log('testingz a');`,
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
    assert.equal(`• Packages in scope: a`, testCLine[1], "Packages in scope");
    assert.equal(
      `• Running test in 1 packages`,
      testCLine[2],
      "Runs only in changed packages"
    );
    assert.ok(
      testCLine[3].startsWith(`a:test: cache miss, executing`),
      "Cache miss in changed package"
    );

    // Check cache hit after another run
    const since2Results = repo.turbo(
      "run",
      ["test", "--since=main", "--stream", "-vvv"],
      options
    );
    const testCLine2 = (since2Results.stdout + since2Results.stderr).split(
      "\n"
    );
    assert.equal(
      `• Packages changed since main: a`,
      testCLine2[0],
      "Calculates changed packages (--since) after a second run"
    );
    assert.equal(
      `• Packages in scope: a`,
      testCLine2[1],
      "Packages in scope after a second run"
    );
    assert.equal(
      `• Running test in 1 packages`,
      testCLine2[2],
      "Runs only in changed packages after a second run"
    );

    assert.ok(
      testCLine2[3].startsWith(`a:test: cache hit, replaying output`),
      "Cache hit in changed package after a second run"
    );

    // Check that hashes are different and trigger a cascade
    repo.commitFiles({
      [path.join("packages", "b", "test.js")]: `console.log('testingz b');`,
    });

    const hashChangeResults = repo.turbo("run", ["test", "--stream"], options);
    const hashChangeResultsOut =
      hashChangeResults.stdout + hashChangeResults.stderr;
    console.log("------------------------------------------------------");
    console.log(hashChangeResultsOut);
    console.log("------------------------------------------------------");
    const testCLine3 = hashChangeResultsOut.split("\n");

    assert.equal(
      `• Packages in scope: a, b, c`,
      testCLine3[0],
      "Packages in scope after a third run"
    );
    assert.equal(
      `• Running test in 3 packages`,
      testCLine3[1],
      "Runs correct number of packages"
    );
    assert.ok(
      testCLine3.findIndex((l) =>
        l.startsWith("a:test: cache miss, executing")
      ) >= 0,
      `A was impacted.`
    );
    assert.ok(
      testCLine3.findIndex((l) =>
        l.startsWith("b:test: cache miss, executing")
      ) >= 0,
      `B was impacted.`
    );
    assert.ok(
      testCLine3.findIndex((l) =>
        l.startsWith("c:test: cache hit, replaying output")
      ) >= 0,
      `C was unchanged`
    );
    repo.cleanup();
  });
}
