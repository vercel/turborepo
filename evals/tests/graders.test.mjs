import assert from "node:assert/strict";
import { spawnSync } from "node:child_process";
import {
  cpSync,
  mkdtempSync,
  readFileSync,
  rmSync,
  writeFileSync
} from "node:fs";
import { tmpdir } from "node:os";
import { join, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import test from "node:test";

const fixtureRoot = resolve(
  fileURLToPath(new URL("../evals", import.meta.url))
);

function command(binary, args, cwd) {
  const result = spawnSync(binary, args, {
    cwd,
    encoding: "utf8",
    env: { ...process.env, CI: "1", TURBO_TELEMETRY_DISABLED: "1" }
  });
  if (result.error) throw result.error;
  return result;
}

function changeConfig(cwd, change) {
  const file = join(cwd, "turbo.json");
  const config = JSON.parse(readFileSync(file, "utf8"));
  change(config);
  writeFileSync(file, `${JSON.stringify(config, null, 2)}\n`);
}

const cases = [
  {
    name: "build-dependency",
    fix(cwd) {
      changeConfig(cwd, (config) => {
        config.tasks.build.dependsOn = ["^build"];
      });
    }
  },
  {
    name: "root-config-cache",
    fix(cwd) {
      changeConfig(cwd, (config) => {
        config.tasks["web#build"] = {
          inputs: ["$TURBO_DEFAULT$", "$TURBO_ROOT$/build-settings.json"],
          outputs: ["dist/**"]
        };
      });
    }
  },
  {
    name: "env-cache",
    fix(cwd) {
      changeConfig(cwd, (config) => {
        config.tasks["web#build"] = {
          env: ["STOREFRONT_API_ORIGIN"],
          outputs: ["dist/**"]
        };
      });
    }
  }
];

for (const { name, fix } of cases) {
  test(`${name} grader fails on starter and passes on a reference fix`, (t) => {
    const directory = mkdtempSync(join(tmpdir(), `turbo-eval-${name}-`));
    t.after(() => rmSync(directory, { recursive: true, force: true }));
    cpSync(join(fixtureRoot, name), directory, { recursive: true });
    // agent-eval supplies this Vitest configuration in the sandbox.
    writeFileSync(
      join(directory, "vitest.config.mjs"),
      "export default { test: { include: ['EVAL.ts'] } };\n"
    );

    const install = command(
      "npm",
      ["ci", "--ignore-scripts", "--no-audit", "--no-fund"],
      directory
    );
    assert.equal(
      install.status,
      0,
      `npm ci: ${install.stdout}\n${install.stderr}`
    );
    const grade = () =>
      command(
        join(directory, "node_modules/.bin/vitest"),
        ["run", "EVAL.ts"],
        directory
      );
    const red = grade();
    assert.notEqual(
      red.status,
      0,
      `starter unexpectedly passed:\n${red.stdout}\n${red.stderr}`
    );

    fix(directory);
    const green = grade();
    assert.equal(
      green.status,
      0,
      `reference fix failed:\n${green.stdout}\n${green.stderr}`
    );
  });
}
