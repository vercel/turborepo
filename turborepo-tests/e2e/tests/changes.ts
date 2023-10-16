import path from "path";
import * as assert from "uvu/assert";
import { getCommandOutputAsArray, getHashFromOutput } from "../helpers";
import { Monorepo } from "../monorepo";
import { PackageManager } from "../types";
import * as uvu from "uvu";

export default function (
  suite: uvu.uvu.Test<uvu.Context>,
  repo: Monorepo,
  pkgManager: PackageManager,
  options: { cwd?: string } = {}
) {
  return suite(`${pkgManager} handles filesystem changes`, async () => {
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
  });
}
