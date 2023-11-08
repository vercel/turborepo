import path from "path";
import * as assert from "uvu/assert";
import * as uvu from "uvu";
import {
  getImmutableInstallForPackageManager,
  getCommandOutputAsArray,
  getLockfileForPackageManager,
} from "../helpers";
import { Monorepo } from "../monorepo";
import { PackageManager } from "../types";

export default function (
  suite: uvu.uvu.Test<uvu.Context>,
  repo: Monorepo,
  pkgManager: PackageManager,
  options: { cwd?: string } = {},
  includePrune: string[] = [],
  excludePrune: string[] = []
) {
  return suite(`${pkgManager} + turbo prune`, async () => {
    const [installCmd, ...installArgs] =
      getImmutableInstallForPackageManager(pkgManager);
    const scope = "a";
    const pruneCommandOutput = getCommandOutputAsArray(
      repo.turbo("prune", [scope], options)
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
      "out/package.json",
      "out/turbo.json",
      `out/${getLockfileForPackageManager(pkgManager)}`,
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
      `Expected ${pkgManager} install --frozen-lockfile to succeed`
    );
  });
}
