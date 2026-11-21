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
  resolveTurboPath,
  resolveWindowsTurboShim
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

function writeFile(filePath: string, contents = "") {
  fs.mkdirSync(path.dirname(filePath), { recursive: true });
  fs.writeFileSync(filePath, contents);
  return filePath;
}

/// Lays out `node_modules/turbo` plus native binary packages the way npm
/// installs them on Windows, and returns the `.bin/turbo.cmd` shim.
function makeWindowsTurboInstall(
  nodeModules: string,
  binaryPackages: Array<string>
) {
  writeFile(path.join(nodeModules, "turbo", "package.json"), "{}");
  for (const binaryPackage of binaryPackages) {
    writeFile(path.join(nodeModules, binaryPackage, "package.json"), "{}");
    writeFile(path.join(nodeModules, binaryPackage, "bin", "turbo.exe"));
  }
  return writeFile(path.join(nodeModules, ".bin", "turbo.cmd"));
}

const win32arm64 = { platform: "win32", arch: "arm64" } as const;
const win32x64 = { platform: "win32", arch: "x64" } as const;

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

test("resolveWindowsTurboShim maps node_modules/.bin shims to the native binary", () => {
  const nodeModules = path.join(tmpdir(), "node_modules");
  const shim = makeWindowsTurboInstall(nodeModules, [
    "@turbo/windows-64",
    "@turbo/windows-arm64"
  ]);
  const binary = (name: string) =>
    fs.realpathSync(path.join(nodeModules, name, "bin", "turbo.exe"));

  assert.equal(
    resolveWindowsTurboShim(shim, win32arm64),
    binary("@turbo/windows-arm64")
  );
  assert.equal(
    resolveWindowsTurboShim(shim, win32x64),
    binary("@turbo/windows-64")
  );
  // PowerShell and extensionless shims sit next to the .cmd one.
  for (const sibling of ["turbo.ps1", "turbo"]) {
    const other = writeFile(path.join(nodeModules, ".bin", sibling));
    assert.equal(
      resolveWindowsTurboShim(other, win32x64),
      binary("@turbo/windows-64")
    );
  }
});

test("resolveWindowsTurboShim follows pnpm links, npm global installs, and legacy names", () => {
  // pnpm: node_modules/turbo links into the virtual store, where the binary
  // package is a sibling of the real turbo package.
  const workspace = tmpdir();
  const store = path.join(workspace, "node_modules", ".pnpm", "turbo@2.0.0");
  makeWindowsTurboInstall(path.join(store, "node_modules"), [
    "@turbo/windows-64"
  ]);
  fs.symlinkSync(
    path.join(store, "node_modules", "turbo"),
    path.join(workspace, "node_modules", "turbo"),
    "junction"
  );
  const pnpmShim = writeFile(
    path.join(workspace, "node_modules", ".bin", "turbo.cmd")
  );
  assert.equal(
    resolveWindowsTurboShim(pnpmShim, win32x64),
    fs.realpathSync(
      path.join(
        store,
        "node_modules",
        "@turbo",
        "windows-64",
        "bin",
        "turbo.exe"
      )
    )
  );

  // npm --global: <prefix>/turbo.cmd, with the binary nested under turbo.
  const prefix = tmpdir();
  const globalTurbo = path.join(prefix, "node_modules", "turbo");
  writeFile(path.join(globalTurbo, "package.json"), "{}");
  const nestedBinary = writeFile(
    path.join(
      globalTurbo,
      "node_modules",
      "@turbo",
      "windows-64",
      "bin",
      "turbo.exe"
    )
  );
  const globalShim = writeFile(path.join(prefix, "turbo.cmd"));
  assert.equal(
    resolveWindowsTurboShim(globalShim, win32x64),
    fs.realpathSync(nestedBinary)
  );

  // Legacy unscoped package name, reached under x64 emulation on arm64.
  const legacy = path.join(tmpdir(), "node_modules");
  const legacyShim = makeWindowsTurboInstall(legacy, ["turbo-windows-64"]);
  assert.equal(
    resolveWindowsTurboShim(legacyShim, win32arm64),
    fs.realpathSync(path.join(legacy, "turbo-windows-64", "bin", "turbo.exe"))
  );
});

test("resolveWindowsTurboShim leaves other paths unchanged", () => {
  const nodeModules = path.join(tmpdir(), "node_modules");
  const shim = makeWindowsTurboInstall(nodeModules, ["@turbo/windows-64"]);
  // Not Windows.
  assert.equal(
    resolveWindowsTurboShim(shim, { platform: "linux", arch: "x64" }),
    shim
  );
  // Already a native binary.
  const exe = path.join(
    nodeModules,
    "@turbo",
    "windows-64",
    "bin",
    "turbo.exe"
  );
  assert.equal(resolveWindowsTurboShim(exe, win32x64), exe);
  // No binary for this architecture.
  assert.equal(
    resolveWindowsTurboShim(shim, { platform: "win32", arch: "ia32" }),
    shim
  );
  // A shim that is not next to a turbo package.
  const stray = writeFile(path.join(tmpdir(), "bin", "turbo.cmd"));
  assert.equal(resolveWindowsTurboShim(stray, win32x64), stray);
});

test(
  "findInstalledTurboLsp probes the native binary behind a Windows .cmd shim",
  { skip: process.platform !== "win32" },
  async () => {
    // Node will not spawn the .cmd shim without a shell (spawn EINVAL), so
    // the probe must run the binary behind it. Stand in for turbo.exe with a
    // link to node.exe that runs `__internal_lsp` from the workspace root.
    const workspace = tmpdir();
    const nodeModules = path.join(workspace, "node_modules");
    const binaryPackage =
      process.arch === "arm64" ? "@turbo/windows-arm64" : "@turbo/windows-64";
    makeWindowsTurboInstall(nodeModules, []);
    const turboExe = path.join(nodeModules, binaryPackage, "bin", "turbo.exe");
    fs.mkdirSync(path.dirname(turboExe), { recursive: true });
    try {
      fs.linkSync(process.execPath, turboExe);
    } catch {
      fs.copyFileSync(process.execPath, turboExe);
    }
    writeFile(
      path.join(workspace, "__internal_lsp"),
      'process.stdout.write("turbo-lsp");'
    );

    const found = await findInstalledTurboLsp({
      workspaceRoot: workspace,
      log: noopLog
    });
    assert.equal(found, fs.realpathSync(turboExe));
  }
);

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
