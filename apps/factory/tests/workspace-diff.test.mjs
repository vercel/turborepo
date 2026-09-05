import assert from "node:assert/strict";
import test from "node:test";

import {
  MAX_WORKSPACE_DIFF_BYTES,
  readWorkspaceDiff,
  WorkspaceDiffTooLargeError
} from "../agent/lib/workspace-diff.ts";

function sandbox({ exitCode = 0, patch = "", stderr = "" } = {}) {
  const commands = [];
  return {
    commands,
    async runCommand(command, args, options) {
      commands.push({ args, command, options });
      return {
        exitCode,
        async stderr() {
          return stderr;
        },
        async stdout() {
          return patch;
        }
      };
    }
  };
}

test("reads tracked and untracked workspace changes against HEAD", async () => {
  const target = sandbox({ patch: "diff --git a/a.ts b/a.ts\n" });
  const patch = await readWorkspaceDiff("eve-sandbox-abc", async (name) => {
    assert.equal(name, "eve-sandbox-abc");
    return target;
  });

  assert.equal(patch, "diff --git a/a.ts b/a.ts\n");
  assert.deepEqual(target.commands, [
    {
      args: [
        "-lc",
        `set -euo pipefail
cd /factory/turborepo
git diff --no-ext-diff --no-color --find-renames HEAD --
while IFS= read -r -d '' path; do
  git diff --no-ext-diff --no-color --no-index -- /dev/null "$path" || [ "$?" -eq 1 ]
done < <(git ls-files --others --exclude-standard -z)`
      ],
      command: "bash",
      options: { timeoutMs: 30_000 }
    }
  ]);
});

test("reports git failures without returning partial output", async () => {
  const target = sandbox({ exitCode: 128, stderr: "not a git repository" });
  await assert.rejects(
    readWorkspaceDiff("eve-sandbox-abc", async () => target),
    /not a git repository/
  );
});

test("rejects diffs too large for the review surface", async () => {
  const target = sandbox({ patch: "x".repeat(MAX_WORKSPACE_DIFF_BYTES + 1) });
  await assert.rejects(
    readWorkspaceDiff("eve-sandbox-abc", async () => target),
    WorkspaceDiffTooLargeError
  );
});
