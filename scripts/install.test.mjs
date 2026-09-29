import assert from "node:assert/strict";
import { createHash } from "node:crypto";
import { execFileSync, spawnSync } from "node:child_process";
import {
  chmod,
  mkdir,
  mkdtemp,
  readFile,
  readdir,
  rm,
  symlink,
  writeFile
} from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { fileURLToPath } from "node:url";
import test from "node:test";

const VERSION = "2.11.5";
const installerPath = fileURLToPath(
  new URL("../apps/docs/public/install", import.meta.url)
);

function currentTarget() {
  const targets = {
    darwin: { arm64: "aarch64-apple-darwin", x64: "x86_64-apple-darwin" },
    linux: {
      arm64: "aarch64-unknown-linux-musl",
      x64: "x86_64-unknown-linux-musl"
    }
  };
  return targets[process.platform]?.[process.arch] ?? null;
}

async function createHarness(root, { corruptArchive = false } = {}) {
  const target = currentTarget();
  assert.ok(
    target,
    `unsupported test host: ${process.platform}/${process.arch}`
  );
  const fakeBin = join(root, "bin");
  const artifactDir = join(root, "artifact");
  const home = join(root, "home");
  await mkdir(fakeBin);
  await mkdir(artifactDir);
  await mkdir(home);

  const executable = join(artifactDir, "turbo");
  const binaryContents = "standalone turbo fixture\n";
  await writeFile(executable, binaryContents);
  await chmod(executable, 0o755);

  const archiveName = `turbo-${VERSION}-${target}.tar.gz`;
  const validArchive = join(root, archiveName);
  execFileSync("tar", ["-czf", validArchive, "-C", artifactDir, "turbo"]);
  const archiveBytes = await readFile(validArchive);
  const digest = createHash("sha256").update(archiveBytes).digest("hex");
  const checksums = join(root, "SHA256SUMS");
  await writeFile(checksums, `${digest}  ${archiveName}\n`);

  let servedArchive = validArchive;
  if (corruptArchive) {
    servedArchive = join(root, "corrupt.tar.gz");
    await writeFile(
      servedArchive,
      Buffer.concat([archiveBytes, Buffer.from("corrupt")])
    );
  }

  const latest = join(root, "latest.json");
  await writeFile(latest, JSON.stringify({ tag_name: `v${VERSION}` }));
  const curlLog = join(root, "curl.log");
  const fakeCurl = join(fakeBin, "curl");
  await writeFile(
    fakeCurl,
    `#!/bin/sh
set -eu
output=
url=
while [ "$#" -gt 0 ]; do
  case "$1" in
    --output|-o) output=$2; shift 2 ;;
    --header|--proto|--proto-redir) shift 2 ;;
    --fail|--show-error|--silent|--location|--tlsv1.2) shift ;;
    *) url=$1; shift ;;
  esac
done
[ -n "$output" ] && [ -n "$url" ] || { echo "unexpected curl arguments" >&2; exit 2; }
printf '%s\\n' "$url" >> "$TURBO_CURL_LOG"
case "$url" in
  */repos/vercel/turborepo/releases/latest) cp "$TURBO_LATEST_FIXTURE" "$output" ;;
  */SHA256SUMS) cp "$TURBO_CHECKSUMS_FIXTURE" "$output" ;;
  */"$TURBO_ARCHIVE_NAME") cp "$TURBO_ARCHIVE_FIXTURE" "$output" ;;
  *) echo "unexpected URL: $url" >&2; exit 22 ;;
esac
`
  );
  await chmod(fakeCurl, 0o755);

  return {
    target,
    archiveName,
    binaryContents,
    checksums,
    curlLog,
    env: {
      ...process.env,
      PATH: `${fakeBin}:/usr/bin:/bin:/usr/sbin:/sbin`,
      HOME: home,
      SHELL: "/bin/zsh",
      GITHUB_PATH: "",
      TURBO_CURL_LOG: curlLog,
      TURBO_NO_MODIFY_PATH: "0",
      TURBO_VERSION: "",
      TURBO_LATEST_FIXTURE: latest,
      TURBO_CHECKSUMS_FIXTURE: checksums,
      TURBO_ARCHIVE_FIXTURE: servedArchive,
      TURBO_ARCHIVE_NAME: archiveName,
      TURBO_INSTALL_DIR: join(root, "install's dir", "bin")
    }
  };
}

function runInstaller(env) {
  return spawnSync("/bin/sh", [installerPath], { encoding: "utf8", env });
}

test("installs the latest stable standalone archive after verifying its manifest digest", async (t) => {
  if (process.platform === "win32") {
    t.skip("POSIX installer tests run on macOS and Linux");
    return;
  }
  const root = await mkdtemp(join(tmpdir(), "turbo-install-test-"));
  try {
    const harness = await createHarness(root);
    const result = runInstaller(harness.env);
    assert.equal(result.status, 0, result.stderr);
    assert.match(result.stdout, new RegExp(`Installed turbo ${VERSION}`));
    assert.equal(
      await readFile(join(harness.env.TURBO_INSTALL_DIR, "turbo"), "utf8"),
      harness.binaryContents
    );
    const profile = await readFile(join(harness.env.HOME, ".zprofile"), "utf8");
    assert.ok(
      profile.includes(
        `Turborepo standalone installer PATH: ${harness.env.TURBO_INSTALL_DIR}`
      )
    );
    const profileCheck = spawnSync(
      "/bin/sh",
      [
        "-c",
        '. "$1"; case ":$PATH:" in *:"$EXPECTED_INSTALL_DIR":*) exit 0 ;; *) exit 1 ;; esac',
        "sh",
        join(harness.env.HOME, ".zprofile")
      ],
      {
        encoding: "utf8",
        env: {
          ...harness.env,
          PATH: "/usr/bin:/bin",
          EXPECTED_INSTALL_DIR: harness.env.TURBO_INSTALL_DIR
        }
      }
    );
    assert.equal(profileCheck.status, 0, profileCheck.stderr);

    const requests = await readFile(harness.curlLog, "utf8");
    assert.match(
      requests,
      /api\.github\.com\/repos\/vercel\/turborepo\/releases\/latest/
    );
    assert.match(
      requests,
      new RegExp(`/v${VERSION}/${harness.archiveName.replaceAll(".", "\\.")}`)
    );
  } finally {
    await rm(root, { recursive: true, force: true });
  }
});

test("adds the install directory to GitHub Actions PATH without editing shell profiles", async (t) => {
  if (process.platform === "win32") {
    t.skip("POSIX installer tests run on macOS and Linux");
    return;
  }
  const root = await mkdtemp(join(tmpdir(), "turbo-install-test-"));
  try {
    const harness = await createHarness(root);
    const githubPath = join(root, "github-path");
    await writeFile(githubPath, "");
    const result = runInstaller({ ...harness.env, GITHUB_PATH: githubPath });
    assert.equal(result.status, 0, result.stderr);
    assert.equal(
      (await readFile(githubPath, "utf8")).trim(),
      harness.env.TURBO_INSTALL_DIR
    );
    assert.deepEqual(await readdir(harness.env.HOME), []);
  } finally {
    await rm(root, { recursive: true, force: true });
  }
});

test("rejects a tampered archive without installing it", async (t) => {
  if (process.platform === "win32") {
    t.skip("POSIX installer tests run on macOS and Linux");
    return;
  }
  const root = await mkdtemp(join(tmpdir(), "turbo-install-test-"));
  try {
    const harness = await createHarness(root, { corruptArchive: true });
    const result = runInstaller(harness.env);
    assert.notEqual(result.status, 0);
    assert.match(result.stderr, /SHA-256 verification failed/);
    await assert.rejects(
      readFile(join(harness.env.TURBO_INSTALL_DIR, "turbo"))
    );
  } finally {
    await rm(root, { recursive: true, force: true });
  }
});

test("refuses an existing PATH shim before downloading or executing it", async (t) => {
  if (process.platform === "win32") {
    t.skip("POSIX installer tests run on macOS and Linux");
    return;
  }
  const root = await mkdtemp(join(tmpdir(), "turbo-install-test-"));
  try {
    const harness = await createHarness(root);
    const targetDirectory = join(root, "existing-target");
    const pathDirectory = join(root, "existing-path");
    const marker = join(root, "existing-turbo-was-run");
    await mkdir(targetDirectory);
    await mkdir(pathDirectory);
    const target = join(targetDirectory, "turbo");
    const shim = join(pathDirectory, "turbo");
    await writeFile(target, `#!/bin/sh\nprintf 'ran' > '${marker}'\n`);
    await chmod(target, 0o755);
    await symlink(target, shim);

    const result = runInstaller({
      ...harness.env,
      PATH: `${pathDirectory}:${harness.env.PATH}`,
      TURBO_VERSION: VERSION
    });
    assert.notEqual(result.status, 0);
    assert.match(result.stderr, /Found an existing turbo on PATH at/);
    assert.match(result.stderr, /Uninstall it using the tool that installed it/);
    assert.match(
      result.stderr,
      /curl -fsSL https:\/\/turborepo\.dev\/install \| sh/
    );
    assert.ok(result.stderr.includes(shim));
    await assert.rejects(readFile(harness.curlLog));
    await assert.rejects(readFile(marker));
    await assert.rejects(
      readFile(join(harness.env.TURBO_INSTALL_DIR, "turbo"))
    );
  } finally {
    await rm(root, { recursive: true, force: true });
  }
});

test("replaces a regular binary in the install directory, including when it is on PATH", async (t) => {
  if (process.platform === "win32") {
    t.skip("POSIX installer tests run on macOS and Linux");
    return;
  }
  const root = await mkdtemp(join(tmpdir(), "turbo-install-test-"));
  try {
    const harness = await createHarness(root);
    const installDirectory = harness.env.TURBO_INSTALL_DIR;
    await mkdir(installDirectory, { recursive: true });
    const destination = join(installDirectory, "turbo");
    await writeFile(destination, "existing executable");
    await chmod(destination, 0o755);
    const result = runInstaller({
      ...harness.env,
      PATH: `${installDirectory}:${harness.env.PATH}`,
      TURBO_VERSION: VERSION
    });
    assert.equal(result.status, 0, result.stderr);
    assert.equal(await readFile(destination, "utf8"), harness.binaryContents);
    assert.match(result.stdout, new RegExp(`Upgraded turbo ${VERSION}`));
  } finally {
    await rm(root, { recursive: true, force: true });
  }
});

test("keeps the old binary when verification fails", async (t) => {
  if (process.platform === "win32") {
    t.skip("POSIX installer tests run on macOS and Linux");
    return;
  }
  const root = await mkdtemp(join(tmpdir(), "turbo-install-test-"));
  try {
    const harness = await createHarness(root, { corruptArchive: true });
    const destination = join(harness.env.TURBO_INSTALL_DIR, "turbo");
    await mkdir(harness.env.TURBO_INSTALL_DIR, { recursive: true });
    await writeFile(destination, "old binary");
    await chmod(destination, 0o755);
    const result = runInstaller({
      ...harness.env,
      PATH: `${harness.env.TURBO_INSTALL_DIR}:${harness.env.PATH}`
    });
    assert.notEqual(result.status, 0);
    assert.match(result.stderr, /SHA-256 verification failed/);
    assert.equal(await readFile(destination, "utf8"), "old binary");
  } finally {
    await rm(root, { recursive: true, force: true });
  }
});

test("refuses a symlink at the install destination before downloading", async (t) => {
  if (process.platform === "win32") {
    t.skip("POSIX installer tests run on macOS and Linux");
    return;
  }
  const root = await mkdtemp(join(tmpdir(), "turbo-install-test-"));
  try {
    const harness = await createHarness(root);
    const target = join(root, "other-turbo");
    await writeFile(target, "keep me");
    await mkdir(harness.env.TURBO_INSTALL_DIR, { recursive: true });
    await symlink(target, join(harness.env.TURBO_INSTALL_DIR, "turbo"));
    const result = runInstaller(harness.env);
    assert.notEqual(result.status, 0);
    assert.match(result.stderr, /is not a regular file; it was left untouched/);
    assert.equal(await readFile(target, "utf8"), "keep me");
    await assert.rejects(readFile(harness.curlLog));
  } finally {
    await rm(root, { recursive: true, force: true });
  }
});

test("rejects install directories that cannot be represented safely in PATH", async (t) => {
  if (process.platform === "win32") {
    t.skip("POSIX installer tests run on macOS and Linux");
    return;
  }
  const root = await mkdtemp(join(tmpdir(), "turbo-install-test-"));
  try {
    const harness = await createHarness(root);
    const result = runInstaller({
      ...harness.env,
      TURBO_INSTALL_DIR: `${root}/unsafe\npath`,
      TURBO_VERSION: VERSION
    });
    assert.notEqual(result.status, 0);
    assert.match(result.stderr, /must not contain a colon or line break/);
    await assert.rejects(readFile(harness.curlLog));
  } finally {
    await rm(root, { recursive: true, force: true });
  }
});

test("installs when SHELL is unset", async (t) => {
  if (process.platform === "win32") {
    t.skip("POSIX installer tests run on macOS and Linux");
    return;
  }
  const root = await mkdtemp(join(tmpdir(), "turbo-install-test-"));
  try {
    const harness = await createHarness(root);
    const env = { ...harness.env, TURBO_VERSION: VERSION };
    delete env.SHELL;
    const result = runInstaller(env);
    assert.equal(result.status, 0, result.stderr);
    assert.equal(
      await readFile(join(env.TURBO_INSTALL_DIR, "turbo"), "utf8"),
      harness.binaryContents
    );
  } finally {
    await rm(root, { recursive: true, force: true });
  }
});

test("rejects unsafe version input before making a request", async (t) => {
  if (process.platform === "win32") {
    t.skip("POSIX installer tests run on macOS and Linux");
    return;
  }
  const root = await mkdtemp(join(tmpdir(), "turbo-install-test-"));
  try {
    const harness = await createHarness(root);
    const result = runInstaller({
      ...harness.env,
      TURBO_VERSION: "../../latest"
    });
    assert.notEqual(result.status, 0);
    assert.match(result.stderr, /invalid release version/);
    await assert.rejects(readFile(harness.curlLog));
  } finally {
    await rm(root, { recursive: true, force: true });
  }
});
