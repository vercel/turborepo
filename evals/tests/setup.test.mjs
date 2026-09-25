import assert from "node:assert/strict";
import { mkdtempSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import test from "node:test";
import { installTurbo, writeAgentsMd } from "../lib/setup.ts";

function sandbox(version = "2.11.5-canary.2") {
  const writes = [];
  const commands = [];
  return {
    writes,
    commands,
    async writeFiles(files) {
      writes.push(files);
    },
    async readFile(path) {
      assert.equal(path, "node_modules/turbo/package.json");
      return JSON.stringify({ version: "2.11.5-canary.2" });
    },
    async runCommand(command, args) {
      commands.push([command, args]);
      return {
        exitCode: 0,
        stdout: command === "node_modules/.bin/turbo" ? `${version}\n` : ""
      };
    }
  };
}

test("setup uploads the local tarball and checks installed docs and binary version", async (t) => {
  const dir = mkdtempSync(join(tmpdir(), "turbo-eval-pack-"));
  const tarball = join(dir, "turbo.tgz");
  writeFileSync(tarball, Buffer.from("fake tarball"));
  const oldValue = process.env.TURBO_EVAL_TARBALL;
  process.env.TURBO_EVAL_TARBALL = tarball;
  t.after(() => {
    if (oldValue === undefined) delete process.env.TURBO_EVAL_TARBALL;
    else process.env.TURBO_EVAL_TARBALL = oldValue;
    rmSync(dir, { recursive: true, force: true });
  });

  const instance = sandbox();
  await installTurbo(instance);
  assert.deepEqual(
    instance.writes[0]["turbo.tgz"],
    Buffer.from("fake tarball")
  );
  assert.deepEqual(
    instance.commands.map(([command]) => command),
    ["npm", "node", "node_modules/.bin/turbo"]
  );
  await assert.rejects(
    () => installTurbo(sandbox("2.11.3")),
    /selected binary 2\.11\.3/
  );
});

test("docs treatment points to bundled docs without changing the baseline", async () => {
  const instance = sandbox();
  await writeAgentsMd(instance);
  assert.match(
    instance.writes[0]["AGENTS.md"],
    /node_modules\/turbo\/docs\/README\.md/
  );
  assert.equal(instance.writes[0]["CLAUDE.md"], "@AGENTS.md\n");
});
