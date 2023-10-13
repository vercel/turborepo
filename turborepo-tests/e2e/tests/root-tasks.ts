import * as assert from "uvu/assert";
import {
  getCommandOutputAsArray,
  getHashFromOutput,
  getCacheItemForHash,
  getCachedLogFilePathForTask,
} from "../helpers";

export default function (suite, repo, pkgManager, options) {
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
