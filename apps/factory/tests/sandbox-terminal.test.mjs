import assert from "node:assert/strict";
import test from "node:test";

import {
  createTerminalSession,
  ensureFxInstalled
} from "../agent/lib/sandbox-terminal.ts";

function sandbox({ hasFx = true } = {}) {
  const commands = [];
  const rootCommands = [];
  return {
    commands,
    rootCommands,
    asUser(name) {
      assert.equal(name, "root");
      return {
        async runCommand(command, args) {
          rootCommands.push({ args, command });
          return { exitCode: 0 };
        }
      };
    },
    async openInteractive() {
      return { token: "secret-token", url: "wss://example.com/pty" };
    },
    async runCommand(command, args) {
      commands.push({ args, command });
      return { exitCode: hasFx ? 0 : 1 };
    }
  };
}

test("terminal sessions open against the server-selected Eve sandbox", async () => {
  let selected;
  const target = sandbox();
  const session = await createTerminalSession(
    "eve-sbx-ses-vercel-abc",
    async (name) => {
      selected = name;
      return target;
    }
  );

  assert.equal(selected, "eve-sbx-ses-vercel-abc");
  assert.deepEqual(session, {
    token: "secret-token",
    url: "wss://example.com/pty"
  });
  assert.equal(target.rootCommands.length, 0);
});

test("older sandboxes install the pinned fx binary on terminal attach", async () => {
  const target = sandbox({ hasFx: false });
  await ensureFxInstalled(target);

  assert.equal(target.rootCommands.length, 1);
  const [{ args, command }] = target.rootCommands;
  assert.equal(command, "bash");
  assert.ok(args[1].includes("/usr/local/bin/fx"));
  assert.ok(args[1].includes("sha256sum --check --strict"));
  assert.ok(args[1].includes("v0.0.5"));
});
