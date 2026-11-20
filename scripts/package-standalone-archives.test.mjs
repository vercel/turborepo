import assert from "node:assert/strict";
import { createHash } from "node:crypto";
import { execFileSync } from "node:child_process";
import { readFileSync } from "node:fs";
import {
  chmod,
  mkdtemp,
  mkdir,
  readFile,
  readdir,
  rm,
  writeFile
} from "node:fs/promises";
import { tmpdir } from "node:os";
import { basename, join } from "node:path";
import { fileURLToPath } from "node:url";
import test from "node:test";

import {
  archiveMembers,
  archiveName,
  packageStandaloneArchives,
  STANDALONE_TARGETS
} from "./package-standalone-archives.mjs";

async function createFixtures(root, targets = STANDALONE_TARGETS) {
  const artifactsDirectory = join(root, "rust-artifacts");
  const contents = new Map();
  for (const { triple, executable } of targets) {
    const artifactDirectory = join(artifactsDirectory, `turbo-${triple}`);
    await mkdir(artifactDirectory, { recursive: true });
    const content = `standalone binary for ${triple}`;
    const path = join(artifactDirectory, executable);
    await writeFile(path, content);
    await chmod(path, 0o755);
    contents.set(triple, content);
  }
  return { artifactsDirectory, contents };
}

test("packages versioned archives with one root-level executable per target", async () => {
  const root = await mkdtemp(join(tmpdir(), "turbo-standalone-archives-"));
  try {
    const { artifactsDirectory, contents } = await createFixtures(root);
    const outputDirectory = join(root, "standalone-artifacts");
    const version = "2.11.5-canary.4";

    const archives = packageStandaloneArchives({
      version,
      artifactsDirectory,
      outputDirectory
    });

    assert.deepEqual(
      archives.map((archive) => basename(archive)),
      STANDALONE_TARGETS.map(({ triple }) => archiveName(version, triple))
    );
    for (const { triple, executable } of STANDALONE_TARGETS) {
      const archive = join(outputDirectory, archiveName(version, triple));
      assert.deepEqual(archiveMembers(archive), [executable]);
      const archivedContent = execFileSync(
        "tar",
        ["-xOzf", archive, executable],
        { encoding: "utf8" }
      );
      assert.equal(archivedContent, contents.get(triple));
    }

    const manifest = (
      await readFile(join(outputDirectory, "SHA256SUMS"), "utf8")
    )
      .trimEnd()
      .split("\n");
    assert.deepEqual(
      manifest,
      STANDALONE_TARGETS.map(({ triple }) => {
        const filename = archiveName(version, triple);
        const archive = join(outputDirectory, filename);
        const digest = createHash("sha256")
          .update(readFileSync(archive))
          .digest("hex");
        return `${digest}  ${filename}`;
      })
    );
  } finally {
    await rm(root, { recursive: true, force: true });
  }
});

test("writes workflow artifacts when given relative directories", async () => {
  const root = await mkdtemp(join(tmpdir(), "turbo-standalone-archives-"));
  try {
    await createFixtures(root);
    const scriptPath = fileURLToPath(
      new URL("./package-standalone-archives.mjs", import.meta.url)
    );
    const version = "2.11.5-canary.4";
    execFileSync(
      process.execPath,
      [scriptPath, version, "rust-artifacts", "standalone-artifacts"],
      { cwd: root, stdio: "ignore" }
    );

    const outputDirectory = join(root, "standalone-artifacts");
    assert.deepEqual(
      (await readdir(outputDirectory)).sort(),
      [
        ...STANDALONE_TARGETS.map(({ triple }) => archiveName(version, triple)),
        "SHA256SUMS"
      ].sort()
    );
    for (const { triple, executable } of STANDALONE_TARGETS) {
      assert.deepEqual(
        archiveMembers(join(outputDirectory, archiveName(version, triple))),
        [executable]
      );
    }
  } finally {
    await rm(root, { recursive: true, force: true });
  }
});

test("fails if a supported target binary is missing", async () => {
  const root = await mkdtemp(join(tmpdir(), "turbo-standalone-archives-"));
  try {
    const { artifactsDirectory } = await createFixtures(
      root,
      STANDALONE_TARGETS.slice(0, -1)
    );
    assert.throws(
      () =>
        packageStandaloneArchives({
          version: "2.11.5-canary.4",
          artifactsDirectory,
          outputDirectory: join(root, "standalone-artifacts")
        }),
      /Missing release binary for x86_64-pc-windows-msvc/
    );
  } finally {
    await rm(root, { recursive: true, force: true });
  }
});

test("rejects versions that cannot be safely used in asset names", () => {
  assert.throws(
    () => archiveName("../../latest", "x86_64-apple-darwin"),
    /Invalid release version/
  );
});
