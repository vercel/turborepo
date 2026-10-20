import assert from "node:assert/strict";
import test from "node:test";
import * as fs from "node:fs";
import * as os from "node:os";
import * as path from "node:path";

import {
  LSP_PROBE_TIMEOUT_MS,
  findExecutableInDirectory,
  findInstalledTurboLsp,
  probeInternalLsp,
  resolveTurboPath
} from "./turbo-discovery";

// These tests spawn real child processes as fake turbo binaries, which
// requires a POSIX shell. The path helpers are covered platform-agnostically.
const cannotSpawn = process.platform === "win32";

function makeFakeTurbo(
  dir: string,
  behavior: { output?: string; sleepSeconds?: number; logFile?: string }
): string {
  fs.mkdirSync(dir, { recursive: true });
  const turboPath = path.join(dir, "turbo");
  const lines = ["#!/bin/sh"];
  if (behavior.logFile) {
    lines.push(`echo probe >> "${behavior.logFile}"`);
  }
  if (behavior.sleepSeconds) {
    lines.push(`sleep ${behavior.sleepSeconds}`);
  }
  if (behavior.output !== undefined) {
    lines.push(`printf '%s' "${behavior.output}"`);
  }
  fs.writeFileSync(turboPath, `${lines.join("\n")}\n`, { mode: 0o755 });
  return turboPath;
}

function tmpdir() {
  return fs.mkdtempSync(path.join(os.tmpdir(), "turbo-discovery-test-"));
}

const noopLog = () => {};

test("findExecutableInDirectory finds plain executables only", () => {
  const dir = tmpdir();
  assert.equal(findExecutableInDirectory(dir, "turbo"), undefined);
  const turboPath = path.join(dir, "turbo");
  fs.writeFileSync(turboPath, "#!/bin/sh\n");
  assert.equal(findExecutableInDirectory(dir, "turbo"), turboPath);
  // A directory named turbo is not an executable candidate.
  const dirOnly = tmpdir();
  fs.mkdirSync(path.join(dirOnly, "turbo"));
  assert.equal(findExecutableInDirectory(dirOnly, "turbo"), undefined);
});

test("resolveTurboPath resolves files, directories, and missing paths", () => {
  const dir = tmpdir();
  const turboPath = path.join(dir, "turbo");
  fs.writeFileSync(turboPath, "#!/bin/sh\n");

  // Direct file path.
  assert.equal(resolveTurboPath(turboPath, undefined, noopLog), turboPath);
  // Directory containing a turbo executable.
  assert.equal(resolveTurboPath(dir, undefined, noopLog), turboPath);
  // Missing path is rejected.
  assert.equal(
    resolveTurboPath(path.join(dir, "nope"), undefined, noopLog),
    undefined
  );
  assert.equal(resolveTurboPath(undefined, undefined, noopLog), undefined);
});

test(
  "probeInternalLsp accepts turbo-lsp output",
  { skip: cannotSpawn },
  async () => {
    const dir = tmpdir();
    const turboPath = makeFakeTurbo(dir, { output: "turbo-lsp" });
    const probe = await probeInternalLsp(turboPath, {});
    assert.deepEqual(probe, { supported: true });
  }
);

test(
  "probeInternalLsp rejects unexpected output",
  { skip: cannotSpawn },
  async () => {
    const dir = tmpdir();
    const turboPath = makeFakeTurbo(dir, { output: "turbo something-else" });
    const probe = await probeInternalLsp(turboPath, {});
    assert.equal(probe.supported, false);
  }
);

test(
  "probeInternalLsp times out on a hanging binary instead of blocking",
  { skip: cannotSpawn },
  async () => {
    const dir = tmpdir();
    const turboPath = makeFakeTurbo(dir, { sleepSeconds: 30 });
    const start = Date.now();
    const probe = await probeInternalLsp(turboPath, {});
    const elapsed = Date.now() - start;
    assert.equal(probe.supported, false);
    assert.ok(
      elapsed < 10_000,
      `probe should have been killed by its timeout, took ${elapsed}ms`
    );
  }
);

test(
  "findInstalledTurboLsp prefers the configured path and probes each binary once",
  { skip: cannotSpawn },
  async () => {
    const dir = tmpdir();
    const logFile = path.join(dir, "probes.log");
    const supported = makeFakeTurbo(path.join(dir, "a"), {
      output: "turbo-lsp",
      logFile
    });
    const workspace = tmpdir();
    fs.mkdirSync(path.join(workspace, "node_modules", ".bin"), {
      recursive: true
    });
    // Same binary reachable through a symlinked .bin entry: must not be
    // probed again after the configured candidate already matched.
    fs.symlinkSync(
      supported,
      path.join(workspace, "node_modules", ".bin", "turbo")
    );

    const found = await findInstalledTurboLsp({
      workspaceRoot: workspace,
      configuredTurboPath: supported,
      log: noopLog
    });
    assert.equal(found, supported);
    assert.equal(
      fs.readFileSync(logFile, "utf8").trim().split("\n").length,
      1,
      "only one probe should have run"
    );
  }
);

test(
  "findInstalledTurboLsp falls back after a timeout and dedupes repeated binaries",
  { skip: cannotSpawn },
  async () => {
    const dir = tmpdir();
    const logFile = path.join(dir, "probes.log");
    // Configured candidate hangs and is killed by the probe timeout.
    const hanging = makeFakeTurbo(path.join(dir, "hang"), {
      sleepSeconds: 30,
      logFile
    });
    const workspace = tmpdir();
    const binDir = path.join(workspace, "node_modules", ".bin");
    fs.mkdirSync(binDir, { recursive: true });
    const supported = makeFakeTurbo(path.join(dir, "b"), {
      output: "turbo-lsp",
      logFile
    });
    fs.symlinkSync(supported, path.join(binDir, "turbo"));

    const start = Date.now();
    const found = await findInstalledTurboLsp({
      workspaceRoot: workspace,
      configuredTurboPath: hanging,
      log: noopLog
    });
    const elapsed = Date.now() - start;

    // The workspace .bin candidate path wins (it points at the supported binary).
    assert.equal(found, path.join(binDir, "turbo"));
    // Hanging probe killed at ~LSP_PROBE_TIMEOUT_MS, then the workspace
    // candidate succeeds. Total must stay well below the hang duration.
    assert.ok(elapsed < 10_000, `took ${elapsed}ms`);
    const probes = fs.readFileSync(logFile, "utf8").trim().split("\n").length;
    // Hanging binary probed once; supported binary probed once via .bin.
    assert.equal(probes, 2);
    assert.ok(LSP_PROBE_TIMEOUT_MS <= 1000);
  }
);

test(
  "findInstalledTurboLsp returns undefined and stops probing when aborted",
  { skip: cannotSpawn },
  async () => {
    const dir = tmpdir();
    const hanging = makeFakeTurbo(path.join(dir, "hang"), {
      sleepSeconds: 30
    });
    const abort = new AbortController();
    const promise = findInstalledTurboLsp({
      configuredTurboPath: hanging,
      signal: abort.signal,
      log: noopLog
    });
    abort.abort();
    const result = await promise.catch((err: unknown) => err);
    assert.ok(
      result === undefined ||
        (result instanceof Error && result.name === "AbortError"),
      `expected undefined or AbortError, got ${String(result)}`
    );
  }
);
