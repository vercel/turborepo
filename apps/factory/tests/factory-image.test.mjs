import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";

import {
  assertFactoryRevision,
  FACTORY_IMAGE_DONE_PHASE,
  FACTORY_IMAGE_SPEC,
  factoryImageFingerprint,
  factoryImagePhases,
  factoryImageScript,
  factoryImageStartCommand,
  parseFactoryImageProgress,
  runFactoryImagePhases
} from "../agent/lib/factory-image.ts";

const COMMIT = "0123456789abcdef0123456789abcdef01234567";

function repositoryFile(relativePath) {
  return readFileSync(new URL(`../../../${relativePath}`, import.meta.url), {
    encoding: "utf8"
  });
}

function fakeRunner(failOn) {
  const commands = [];
  return {
    commands,
    run(command) {
      commands.push(command);
      const failing = failOn !== undefined && command.includes(failOn);
      return Promise.resolve({
        exitCode: failing ? 3 : 0,
        stderr: failing ? "boom" : "",
        stdout: ""
      });
    }
  };
}

test("the image spec pins the versions the repository requires", () => {
  const toolchain = repositoryFile("rust-toolchain.toml");
  assert.match(
    toolchain,
    new RegExp(`channel = "${FACTORY_IMAGE_SPEC.rustChannel}"`),
    "rust-toolchain.toml disagrees with the factory image spec"
  );
  for (const component of FACTORY_IMAGE_SPEC.rustComponents) {
    assert.match(toolchain, new RegExp(`"${component}"`));
  }

  const rootPackage = JSON.parse(repositoryFile("package.json"));
  assert.equal(
    rootPackage.packageManager,
    `pnpm@${FACTORY_IMAGE_SPEC.pnpmVersion}`
  );
  assert.equal(rootPackage.engines.node, `${FACTORY_IMAGE_SPEC.nodeMajor}.x`);
});

test("the image spec pins the versions CI sets up", () => {
  assert.match(
    repositoryFile(".github/actions/setup-zig/action.yml"),
    new RegExp(`default: "${FACTORY_IMAGE_SPEC.zigVersion}"`)
  );
  const capnproto = repositoryFile(
    ".github/actions/setup-capnproto/action.yml"
  );
  assert.ok(
    capnproto.includes(`capnproto-c++-${FACTORY_IMAGE_SPEC.capnprotoVersion}`)
  );
  assert.ok(capnproto.includes(FACTORY_IMAGE_SPEC.capnprotoSha256));
  assert.match(
    repositoryFile(".github/actions/setup-protoc/action.yml"),
    new RegExp(
      `default: "${FACTORY_IMAGE_SPEC.protocVersion.split(".")[0]}\\.x"`
    )
  );
});

test("the devcontainer image tracks the factory image spec", () => {
  const dockerfile = repositoryFile(".devcontainer/Dockerfile");
  for (const [key, value] of [
    ["RUST_VERSION", FACTORY_IMAGE_SPEC.rustChannel],
    ["NODE_MAJOR", String(FACTORY_IMAGE_SPEC.nodeMajor)],
    ["PNPM_VERSION", FACTORY_IMAGE_SPEC.pnpmVersion],
    ["PROTOC_VERSION", FACTORY_IMAGE_SPEC.protocVersion],
    ["ZIG_VERSION", FACTORY_IMAGE_SPEC.zigVersion]
  ]) {
    assert.match(
      dockerfile,
      new RegExp(`^ENV ${key}=${value.replaceAll(".", "\\.")}$`, "m"),
      `.devcontainer/Dockerfile has drifted for ${key}`
    );
  }
  for (const tool of FACTORY_IMAGE_SPEC.performanceTools) {
    assert.ok(
      dockerfile.includes(tool.crate),
      `.devcontainer/Dockerfile is missing ${tool.crate}`
    );
  }
});

test("phases install every tool an agent needs", () => {
  const phases = factoryImagePhases({ revision: COMMIT });
  assert.deepEqual(
    phases.map((phase) => phase.id),
    [
      "system-packages",
      "node",
      "pnpm",
      "rust",
      "protoc",
      "zig",
      "checkout",
      "node-modules",
      "cargo-registry",
      "performance-tools",
      "verify"
    ]
  );

  const script = phases.map((phase) => phase.script).join("\n");
  for (const expected of [
    "build-essential",
    "capnproto",
    "lld",
    `pnpm@${FACTORY_IMAGE_SPEC.pnpmVersion}`,
    FACTORY_IMAGE_SPEC.rustChannel,
    FACTORY_IMAGE_SPEC.protocVersion,
    FACTORY_IMAGE_SPEC.zigVersion,
    "pnpm install --frozen-lockfile",
    "cargo fetch --locked",
    "hyperfine",
    "cargo-bloat",
    "twiggy",
    COMMIT
  ]) {
    assert.ok(script.includes(expected), `phases are missing ${expected}`);
  }
});

test("the warm build is opt-in", () => {
  const lean = factoryImagePhases({ revision: COMMIT });
  const warm = factoryImagePhases({ revision: COMMIT, warmBuild: true });
  assert.ok(!lean.some((phase) => phase.id === "warm-build"));
  assert.ok(warm.some((phase) => phase.id === "warm-build"));
  assert.equal(warm.at(-1)?.id, "verify");
});

test("only commit-shaped revisions reach the shell", () => {
  assert.equal(assertFactoryRevision(COMMIT), COMMIT);
  assert.equal(assertFactoryRevision("main"), "main");
  for (const revision of [
    "main; rm -rf /",
    "$(whoami)",
    "refs/heads/main",
    "../../etc/passwd",
    ""
  ]) {
    assert.throws(() => assertFactoryRevision(revision), /Invalid Turborepo/);
  }
});

test("the detached script records its phase and exit code", () => {
  const script = factoryImageScript({ revision: COMMIT, warmBuild: true });
  assert.match(script, /^#!\/usr\/bin\/env bash/);
  assert.ok(script.includes("set -euo pipefail"));
  assert.ok(script.includes("trap on_exit EXIT"));
  assert.ok(
    script.includes(`FACTORY_STATE=${FACTORY_IMAGE_SPEC.stateDirectory}`)
  );
  assert.ok(script.includes('> "$FACTORY_STATE/exit"'));
  assert.ok(script.includes(`factory_phase ${FACTORY_IMAGE_DONE_PHASE}`));
  for (const phase of factoryImagePhases({
    revision: COMMIT,
    warmBuild: true
  })) {
    assert.ok(script.includes(`factory_phase ${phase.id}`));
  }
});

test("starting provisioning reuses a live detached process", () => {
  const command = factoryImageStartCommand();
  assert.ok(command.includes("kill -0"));
  assert.ok(command.includes("already running"));
  assert.ok(command.includes('printf \'%s\\n\' "$!" > "$pid_file"'));
});

test("the fingerprint tracks the toolchain, not the commit", () => {
  const fingerprint = factoryImageFingerprint();
  assert.match(fingerprint, /^[0-9a-f]{16}$/);
  assert.equal(fingerprint, factoryImageFingerprint(FACTORY_IMAGE_SPEC));
  assert.notEqual(
    fingerprint,
    factoryImageFingerprint({
      ...FACTORY_IMAGE_SPEC,
      rustChannel: "nightly-2020-01-01"
    })
  );
  assert.notEqual(
    fingerprint,
    factoryImageFingerprint({ ...FACTORY_IMAGE_SPEC, pnpmVersion: "9.0.0" })
  );
});

test("progress parsing separates markers from output", () => {
  const progress = parseFactoryImageProgress(
    [
      "phase=node-modules",
      "exit=-",
      "--factory-warnings--",
      "could not install twiggy",
      "--factory-manifest--",
      '{"node":"v24.0.0","pnpm":"10.28.0"}',
      "--factory-log--",
      "installing",
      "still installing"
    ].join("\n")
  );
  assert.equal(progress.phase, "node-modules");
  assert.equal(progress.exitCode, null);
  assert.deepEqual(progress.warnings, ["could not install twiggy"]);
  assert.deepEqual(progress.manifest, {
    node: "v24.0.0",
    pnpm: "10.28.0"
  });
  assert.equal(progress.logTail, "installing\nstill installing");

  const finished = parseFactoryImageProgress(
    ["phase=done", "exit=0", "--factory-warnings--", "--factory-log--"].join(
      "\n"
    )
  );
  assert.equal(finished.exitCode, 0);
  assert.deepEqual(finished.warnings, []);
  assert.equal(finished.manifest, null);

  const failed = parseFactoryImageProgress("phase=rust\nexit=3\n");
  assert.equal(failed.exitCode, 3);
});

test("the sequential runner reports the phase that failed", async () => {
  const runner = fakeRunner();
  const seen = [];
  await runFactoryImagePhases(runner, { revision: COMMIT }, (phase) =>
    seen.push(phase.id)
  );
  assert.equal(seen.length, 11);
  assert.equal(seen[0], "system-packages");
  assert.equal(runner.commands.length, 11);
  assert.ok(runner.commands.every((command) => command.includes("as_root()")));

  await assert.rejects(
    runFactoryImagePhases(fakeRunner("cargo fetch --locked"), {
      revision: COMMIT
    }),
    /phase "cargo-registry".*exit code 3/s
  );
});
