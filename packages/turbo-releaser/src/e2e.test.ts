import assert from "node:assert/strict";
import { test } from "node:test";
import path from "node:path";
import { tmpdir, arch as osArch, platform } from "node:os";
import { access, mkdir, realpath, rm, writeFile } from "node:fs/promises";
import { execFileSync, execSync } from "node:child_process";
import { constants } from "node:fs";
import operations from "./operations";
import native from "./native";
import type { SupportedOS, SupportedArch } from "./types";

test("produces installable archive", async () => {
  const tempDir = path.join(await realpath(tmpdir()), "turboreleaser-e2e-test");
  await rm(tempDir, { recursive: true, force: true });
  await mkdir(tempDir, { recursive: true });

  // Need to match actual values otherwise npm will refuse to run
  const os = platform() as SupportedOS;
  const arch = osArch() as SupportedArch;
  const humanArch = native.archToHuman[arch];

  // make a fake turbo binary
  const platformPath = `dist-${os}-${arch}`;
  await mkdir(path.join(tempDir, platformPath));
  await writeFile(
    path.join(tempDir, platformPath, "turbo"),
    "#!/bin/bash\necho Invoked fake turbo!"
  );

  const tarPath = await operations.packPlatform({
    platform: { os, arch },
    version: "0.1.2",
    srcDir: tempDir,
    packagePrefix: "@turbo"
  });
  assert.ok(path.isAbsolute(tarPath));

  // Make a fake repo to install the tarball in
  const fakeRepo = path.join(tempDir, "fake-repo");
  await mkdir(fakeRepo);
  await writeFile(
    path.join(fakeRepo, "package.json"),
    JSON.stringify({ name: "fake-repo" })
  );
  execFileSync("npm", ["install", tarPath], { cwd: fakeRepo });

  // The scoped platform package installs the binary at a known path.
  // In production, the main `turbo` package resolves it via require.resolve.
  const binaryPath = path.join(
    fakeRepo,
    "node_modules",
    "@turbo",
    `${os}-${humanArch}`,
    "bin",
    "turbo"
  );
  await access(binaryPath, constants.X_OK);

  const output = execSync(binaryPath, {
    stdio: "pipe",
    cwd: fakeRepo,
    encoding: "utf-8"
  });
  assert.equal(output, "Invoked fake turbo!\n");
});
