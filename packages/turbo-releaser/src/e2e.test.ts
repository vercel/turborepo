import assert from "node:assert/strict";
import { test } from "node:test";
import path from "node:path";
import { tmpdir, arch as osArch, platform } from "node:os";
import { mkdir, realpath, rm, writeFile } from "node:fs/promises";
import { execSync } from "node:child_process";
import operations from "./operations";
import type { SupportedOS, SupportedArch } from "./types";

test("produces installable archive", async () => {
  const tempDir = path.join(await realpath(tmpdir()), "turboreleaser-e2e-test");
  await rm(tempDir, { recursive: true, force: true });
  await mkdir(tempDir, { recursive: true });

  // Need to match actual values otherwise npm will refuse to run
  const os = platform() as SupportedOS;
  const arch = osArch() as SupportedArch;

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
  });
  assert.ok(path.isAbsolute(tarPath));

  // Make a fake repo to install the tarball in
  const fakeRepo = path.join(tempDir, "fake-repo");
  await mkdir(fakeRepo);
  await writeFile(
    path.join(fakeRepo, "package.json"),
    JSON.stringify({
      name: "fake-repo",
      scripts: { "test-turbo-install": "turbo" },
    })
  );
  execSync(`npm install ${tarPath}`, { cwd: fakeRepo });
  const output = execSync("npm run test-turbo-install", {
    stdio: "pipe",
    cwd: fakeRepo,
    encoding: "utf-8",
  });
  assert.equal(
    output,
    "\n> test-turbo-install\n> turbo\n\nInvoked fake turbo!\n"
  );
});
