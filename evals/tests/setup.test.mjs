import assert from "node:assert/strict";
import { mkdtempSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import test from "node:test";
import {
  installTurbo,
  invalidateGuidanceContamination,
  proveGuidanceIsolation,
  writeAgentsMd
} from "../lib/setup.ts";

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
  assert.equal(instance.writes[0]["GEMINI.md"], "@AGENTS.md\n");
  assert.doesNotMatch(
    instance.writes[0]["AGENTS.md"],
    /BEGIN:turborepo-agent-rules/
  );
});

test("post-run isolation leaves clean runs unchanged", () => {
  for (const files of [
    {},
    { generatedFiles: {}, deletedFiles: [] },
    {
      generatedFiles: {
        "turbo.json": Buffer.from('{"agentGuidance":false,"tasks":{}}'),
        "AGENTS.md": Buffer.from("Read the bundled docs"),
        "apps/web/turbo.json": Buffer.from('{"extends":["//"]}'),
        "notes.md": Buffer.from("<!-- BEGIN:turborepo-agent-rules -->")
      },
      deletedFiles: ["apps/web/turbo.json", "AGENTS.md"]
    },
    {
      generatedFiles: {
        "turbo.jsonc": Buffer.from(`{
          // Keep automatic instructions disabled.
          "agentGuidance": false,
          "url": "https://example.com//docs", /* a block comment */
          "tasks": {},
        }`)
      }
    }
  ]) {
    const runData = { result: { status: "passed", duration: 1 }, ...files };
    assert.equal(invalidateGuidanceContamination(runData), runData);
  }
});

test("post-run isolation invalidates root configs without an explicit false", () => {
  for (const path of ["turbo.json", "turbo.jsonc"]) {
    for (const content of [
      '{"tasks":{}}',
      '{"agentGuidance":true}',
      '{"agentGuidance":"false"}',
      '{"tasks":{"agentGuidance":false}}',
      '{/* "agentGuidance": false */ "tasks":{}}',
      '{\n// "agentGuidance": false\n"tasks":{}}',
      '{"note":"\\"agentGuidance\\": false"}',
      '{"agentGuidance":false,"agentGuidance":true}',
      '{"agentGuidance":false',
      "null"
    ]) {
      const runData = {
        result: {
          status: "passed",
          duration: 1,
          metadata: { retained: "value" },
          outputPaths: { eval: "./outputs/eval.txt" }
        },
        outputContent: { eval: "grader passed" },
        generatedFiles: { [path]: Buffer.from(content) }
      };
      const updated = invalidateGuidanceContamination(runData);
      assert.equal(updated.result.status, "failed", `${path}: ${content}`);
      assert.equal(typeof updated.result.metadata.invalidAttempt, "string");
      assert.match(updated.result.error, /agentGuidance.*false/);
      assert.ok(updated.result.error.includes(path));
      assert.equal(updated.result.metadata.retained, "value");
      assert.equal(updated.outputContent, runData.outputContent);
      assert.equal(updated.result.outputPaths, runData.result.outputPaths);
      assert.equal(runData.result.status, "passed");
    }
  }
});

test("post-run isolation invalidates deletion of either root config", () => {
  for (const path of ["turbo.json", "turbo.jsonc"]) {
    const updated = invalidateGuidanceContamination({
      result: { status: "passed", duration: 1 },
      deletedFiles: [path]
    });
    assert.equal(updated.result.status, "failed");
    assert.ok(updated.result.metadata.invalidAttempt.includes(path));
    assert.match(updated.result.error, /deleted/);
  }
});

test("post-run isolation finds automatic guidance in instruction files at any depth", () => {
  for (const name of ["AGENTS.md", "CLAUDE.md", "GEMINI.md"]) {
    for (const prefix of ["", "apps/web/nested/"]) {
      const path = `${prefix}${name}`;
      const updated = invalidateGuidanceContamination({
        result: { status: "failed", duration: 1, error: "grader failed" },
        generatedFiles: {
          [path]: Buffer.from(
            "<!-- BEGIN:turborepo-agent-rules -->\nAutomatic guidance"
          )
        }
      });
      assert.equal(updated.result.status, "failed");
      assert.ok(updated.result.metadata.invalidAttempt.includes(path));
      assert.match(updated.result.error, /automatic Turbo guidance/);
      assert.match(updated.result.error, /grader failed/);
    }
  }
});

test("isolation proof surfaces setup failures before the agent runs", async () => {
  const instance = sandbox();
  await proveGuidanceIsolation(instance);
  assert.equal(instance.writes.length, 0);
  assert.equal(instance.commands[0][0], "node");
  instance.runCommand = async () => ({
    exitCode: 1,
    stderr: "fixture must opt out"
  });
  await assert.rejects(
    () => proveGuidanceIsolation(instance),
    /Guidance isolation proof failed \(1\): fixture must opt out/
  );
});
