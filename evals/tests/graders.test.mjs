import assert from "node:assert/strict";
import { spawnSync } from "node:child_process";
import {
  cpSync,
  existsSync,
  mkdirSync,
  mkdtempSync,
  readFileSync,
  readdirSync,
  rmSync,
  writeFileSync
} from "node:fs";
import { tmpdir } from "node:os";
import { join, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import test from "node:test";
import { proveGuidanceIsolation, writeAgentsMd } from "../lib/setup.ts";
import jitCodegen from "./references/jit-codegen.mjs";
import taskEntrypoints from "./references/task-entrypoints.mjs";

const fixtureRoot = resolve(
  fileURLToPath(new URL("../evals", import.meta.url))
);

function command(binary, args, cwd, env = {}) {
  const result = spawnSync(binary, args, {
    cwd,
    encoding: "utf8",
    timeout: 180_000,
    maxBuffer: 4 * 1024 * 1024,
    env: { ...process.env, CI: "1", TURBO_TELEMETRY_DISABLED: "1", ...env }
  });
  if (result.error) throw result.error;
  return result;
}

function mustRun(binary, args, cwd, env) {
  const result = command(binary, args, cwd, env);
  assert.equal(
    result.status,
    0,
    `${binary} ${args.join(" ")}: ${result.stdout}\n${result.stderr}`
  );
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
  },
  {
    ...jitCodegen,
    failurePattern:
      /root build command must remain unchanged|A → B → B → A|repeated B must restore|editing render.cjs must invalidate|a web source edit must leave UI cached/
  },
  {
    ...taskEntrypoints,
    failurePattern: /test must run only|normal build must retain docs/
  }
];

function prepare(t, name) {
  const directory = mkdtempSync(join(tmpdir(), `turbo-eval-${name}-`));
  t.after(() => rmSync(directory, { recursive: true, force: true }));
  cpSync(join(fixtureRoot, name), directory, { recursive: true });
  // Match agent-eval's initGitAndCommit, including its overwritten .gitignore.
  writeFileSync(
    join(directory, ".gitignore"),
    "node_modules/\n__agent_eval__/\n"
  );
  mustRun("git", ["init", "-q"], directory);
  mustRun("git", ["add", "."], directory);
  // This commit is only in the disposable sandbox, never in the user's checkout.
  mustRun(
    "git",
    [
      "-c",
      "user.name=Eval fixture",
      "-c",
      "user.email=fixture@example.test",
      "-c",
      "commit.gpgsign=false",
      "commit",
      "-qm",
      "Sandbox starter"
    ],
    directory
  );
  mustRun(
    "npm",
    ["ci", "--ignore-scripts", "--no-audit", "--no-fund"],
    directory
  );
  mkdirSync(join(directory, "__agent_eval__"));
  // agent-eval supplies a Vitest config after the agent finishes.
  writeFileSync(
    join(directory, "vitest.config.mjs"),
    "export default { test: { include: ['EVAL.ts'] } };\n"
  );
  return directory;
}

function localSandbox(directory) {
  return {
    async runCommand(binary, args) {
      const result = command(binary, args, directory);
      return {
        exitCode: result.status,
        stdout: result.stdout,
        stderr: result.stderr
      };
    },
    async writeFiles(files) {
      for (const [path, content] of Object.entries(files)) {
        writeFileSync(join(directory, path), content);
      }
    }
  };
}

test("every discovered fixture has a grader case and opts out of automatic guidance", () => {
  const names = readdirSync(fixtureRoot, { withFileTypes: true })
    .filter((entry) => entry.isDirectory())
    .map((entry) => entry.name)
    .sort();
  assert.deepEqual(names, cases.map(({ name }) => name).sort());
  for (const name of names) {
    const directory = join(fixtureRoot, name);
    const config = JSON.parse(
      readFileSync(join(directory, "turbo.json"), "utf8")
    );
    assert.equal(
      config.agentGuidance,
      false,
      `${name} must opt out of automatic guidance`
    );
    const manifest = JSON.parse(
      readFileSync(join(directory, "package.json"), "utf8")
    );
    assert.equal(
      manifest.devDependencies.turbo,
      "2.11.7",
      `${name} must use the tested Turbo version`
    );
    for (const file of ["PROMPT.md", "EVAL.ts", "package-lock.json"]) {
      assert.ok(
        existsSync(join(directory, file)),
        `${name} is missing ${file}`
      );
    }
    for (const file of ["AGENTS.md", "CLAUDE.md", "GEMINI.md"]) {
      assert.equal(
        existsSync(join(directory, file)),
        false,
        `${name} leaks instructions into baseline`
      );
    }
  }
});

for (const { name, fix, nearMisses = {}, failurePattern } of cases) {
  test(`${name} grader rejects starter and near misses, accepts reference`, async (t) => {
    const variants = [
      ["starter", undefined],
      ["reference", fix],
      ...Object.entries(nearMisses)
    ];
    for (const [variant, apply] of variants) {
      await t.test(variant, (t) => {
        const directory = prepare(t, name);
        if (apply) apply(directory);
        const reportPath = join(directory, "__agent_eval__/grade.json");
        const result = command(
          join(directory, "node_modules/.bin/vitest"),
          ["run", "EVAL.ts", "--reporter=json", `--outputFile=${reportPath}`],
          directory
        );
        const output = `${result.stdout}\n${result.stderr}`;
        assert.ok(
          existsSync(reportPath),
          `grader produced no report: ${output}`
        );
        const report = JSON.parse(readFileSync(reportPath, "utf8"));
        assert.ok(report.numTotalTests > 0, `grader ran no tests: ${output}`);
        if (variant === "reference") {
          assert.equal(
            result.status,
            0,
            `reference failed: ${output}\n${JSON.stringify(report)}`
          );
          assert.equal(report.success, true);
        } else {
          assert.equal(
            result.status,
            1,
            `bad solution unexpectedly passed: ${output}`
          );
          const failures = report.testResults
            .flatMap((suite) => suite.assertionResults)
            .filter((assertion) => assertion.status === "failed")
            .flatMap((assertion) => assertion.failureMessages);
          const behavioralFailures = failures
            .filter(
              (failure) =>
                /AssertionError/.test(failure) &&
                !/Cannot find module|spawn .*ENOENT|Test timed out/.test(
                  failure
                )
            )
            .join("\n");
          assert.ok(
            behavioralFailures,
            `red must include a behavioral assertion: ${failures.join("\n")}`
          );
          if (failurePattern) assert.match(behavioralFailures, failurePattern);
        }
      });
    }
  });
}

test("real Turbo preserves both arms; removing the opt-out contaminates baseline", async (t) => {
  const directory = prepare(t, "build-dependency");
  const sandbox = localSandbox(directory);
  await proveGuidanceIsolation(sandbox);
  await proveGuidanceIsolation(sandbox);
  for (const file of ["AGENTS.md", "CLAUDE.md", "GEMINI.md"]) {
    assert.equal(existsSync(join(directory, file)), false);
  }
  await writeAgentsMd(sandbox);
  const treatment = readFileSync(join(directory, "AGENTS.md"), "utf8");
  await proveGuidanceIsolation(sandbox);
  await proveGuidanceIsolation(sandbox);
  assert.equal(readFileSync(join(directory, "AGENTS.md"), "utf8"), treatment);
  for (const file of ["AGENTS.md", "CLAUDE.md", "GEMINI.md"]) {
    rmSync(join(directory, file));
  }
  changeConfig(directory, (config) => {
    delete config.agentGuidance;
  });
  await assert.rejects(
    () => proveGuidanceIsolation(sandbox),
    /must set "agentGuidance": false/
  );
  mustRun("node_modules/.bin/turbo", ["ls"], directory, { CLAUDECODE: "1" });
  assert.match(
    readFileSync(join(directory, "AGENTS.md"), "utf8"),
    /<!-- BEGIN:turborepo-agent-rules -->/
  );
});
