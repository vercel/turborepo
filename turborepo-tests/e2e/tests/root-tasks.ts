import * as uvu from "uvu";
import * as assert from "uvu/assert";
import { getCommandOutputAsArray, getHashFromOutput } from "../helpers";
import { Monorepo } from "../monorepo";
import { PackageManager } from "../types";

export default function (
  suite: uvu.uvu.Test<uvu.Context>,
  repo: Monorepo,
  pkgManager: PackageManager,
  options?: { cwd?: string }
) {
  return suite(`${pkgManager} runs root tasks`, async () => {
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
}
