import assert from "node:assert/strict";
import { spawnSync } from "node:child_process";
import { readFileSync } from "node:fs";
import test from "node:test";

import {
  factorySandboxArgs,
  factorySandboxCommand
} from "../lib/factory-sandbox-command.ts";

const source = (path) => readFileSync(new URL(path, import.meta.url), "utf8");
const credentialExpressions =
  ' --env "GH_TOKEN=$(gh auth token)"' +
  ' --env "AI_GATEWAY_API_KEY=${AI_GATEWAY_API_KEY:?Set AI_GATEWAY_API_KEY locally first}"';

function runCommand(apiKey) {
  return spawnSync(
    "bash",
    [
      "--noprofile",
      "--norc",
      "-c",
      `gh() { printf '%s' 'test-github-token'; }
       sandbox() { printf '%s\\n' "$@"; }
       ${factorySandboxCommand("snap_published")}`
    ],
    {
      encoding: "utf8",
      env: { PATH: process.env.PATH, AI_GATEWAY_API_KEY: apiKey }
    }
  );
}

test("opens the published snapshot with an interactive shell and factory resources", () => {
  assert.deepEqual(factorySandboxArgs("snap_published"), [
    "create",
    "--snapshot",
    "snap_published",
    "--vcpus",
    "8",
    "--timeout",
    "45m",
    "--connect"
  ]);
  assert.equal(
    factorySandboxCommand("snap_published"),
    "sandbox create --snapshot snap_published --vcpus 8 --timeout 45m --connect" +
      credentialExpressions
  );
});

test("does not create a sandbox from scratch when no image is published", () => {
  assert.throws(() => factorySandboxArgs(""), /No factory image/);
  assert.throws(() => factorySandboxCommand("  "), /No factory image/);
});

test("quotes snapshot IDs in the copyable shell command", () => {
  assert.equal(
    factorySandboxCommand("snap_'$(echo unexpected)"),
    "sandbox create --snapshot 'snap_'\\''$(echo unexpected)' --vcpus 8 --timeout 45m --connect" +
      credentialExpressions
  );
  assert.equal(factorySandboxArgs("snap_$value")[2], "snap_$value");
});

test("credentials expand locally and remain single arguments", () => {
  const apiKey = 'test-key with spaces; $(echo unexpected) "quoted"';
  const result = runCommand(apiKey);
  assert.equal(result.status, 0, result.stderr);
  assert.deepEqual(result.stdout.trimEnd().split("\n"), [
    ...factorySandboxArgs("snap_published"),
    "--env",
    "GH_TOKEN=test-github-token",
    "--env",
    `AI_GATEWAY_API_KEY=${apiKey}`
  ]);
});

test("missing or empty AI Gateway key prevents sandbox creation", () => {
  for (const apiKey of [undefined, ""]) {
    const result = runCommand(apiKey);
    assert.notEqual(result.status, 0);
    assert.equal(result.stdout, "");
    assert.match(result.stderr, /Set AI_GATEWAY_API_KEY locally first/);
  }
});

test("the command is exposed without a factory CLI shortcut", () => {
  const cli = source("../scripts/factory.mjs");
  assert.doesNotMatch(cli, /command === "sandbox"|factorySandboxArgs/);
  const card = source("../app/factory-sandbox-command.tsx");
  assert.doesNotMatch(card, /factory sandbox|FACTORY_URL/);
});

test("latest-image lookup does not reconcile or rebuild images", () => {
  const route = source("../app/api/factory-image/current/route.ts");
  assert.match(route, /readFactoryImagePointer\(\)/);
  assert.match(route, /"cache-control": "no-store"/);
  assert.doesNotMatch(
    route,
    /reconcileFactoryImageBuilds|triggerFactoryImageBuild/
  );
  const homeCard = source("../app/latest-factory-sandbox-command.tsx");
  assert.match(homeCard, /fetch\("\/api\/factory-image\/current"/);
});

test("both pages expose the command using the published pointer", () => {
  const home = source("../app/page.tsx");
  assert.match(home, /readFactoryImagePointer\(\)/);
  assert.match(
    home,
    /LatestFactorySandboxCommand\s+initialSnapshotId=\{pointer\?\.snapshotId/
  );
  const image = source("../app/factory-image.tsx");
  assert.match(
    image,
    /FactorySandboxCommand snapshotId=\{pointer\?\.snapshotId/
  );
  const card = source("../app/factory-sandbox-command.tsx");
  assert.match(card, /navigator\.clipboard\.writeText\(command\)/);
  assert.match(card, /disabled=\{!command\}/);
});
