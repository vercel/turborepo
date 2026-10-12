import assert from "node:assert/strict";
import { createHash } from "node:crypto";
import { execFileSync, spawnSync } from "node:child_process";
import {
  chmod,
  lstat,
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

async function createHarness(
  root,
  { corruptArchive = false, hookSupport = true } = {}
) {
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
  // Releases before the shell hook reject `turbo hook <shell>`.
  const binaryContents = hookSupport
    ? `#!/bin/sh\n[ "$1" = hook ] || exit 1\nprintf 'TURBO_HOOKED=%s\\n' "$2"\n`
    : "#!/bin/sh\nexit 1\n";
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

function runInstaller(env, args = []) {
  return spawnSync("/bin/sh", [installerPath, ...args], {
    encoding: "utf8",
    env
  });
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

const HOOK_MARKER = "# Turborepo standalone installer shell hook";

async function readIfExists(path) {
  try {
    return await readFile(path, "utf8");
  } catch {
    return null;
  }
}

function hookLines(text) {
  return (text ?? "").split("\n").filter((line) => line.includes(HOOK_MARKER));
}

async function withHarness(t, options, body) {
  if (process.platform === "win32") {
    t.skip("POSIX installer tests run on macOS and Linux");
    return;
  }
  const root = await mkdtemp(join(tmpdir(), "turbo-install-test-"));
  try {
    const harness = await createHarness(root, options);
    await body(root, { ...harness.env, TURBO_VERSION: VERSION });
  } finally {
    await rm(root, { recursive: true, force: true });
  }
}

test("adds the shell hook to zsh once and updates only its own line", (t) =>
  withHarness(t, {}, async (root, env) => {
    // Upgrades must write through a dotfile manager's symlink.
    const dotfile = join(root, "dotfiles-zshrc");
    const stale = `eval "$('/old/turbo' hook zsh)" ${HOOK_MARKER}`;
    await writeFile(dotfile, `# mine\n${stale}\n# also mine\n${stale}\n`);
    await symlink(dotfile, join(env.HOME, ".zshrc"));
    for (let run = 0; run < 2; run++) {
      const result = runInstaller(env);
      assert.equal(result.status, 0, result.stderr);
      assert.match(result.stdout, /Added the turbo shell hook/);
    }
    assert.ok((await lstat(join(env.HOME, ".zshrc"))).isSymbolicLink());
    const lines = (await readFile(dotfile, "utf8")).split("\n");
    assert.equal(lines[0], "# mine");
    assert.equal(lines[2], "# also mine");
    assert.deepEqual(hookLines(lines.join("\n")), [lines[1]]);
    assert.deepEqual(
      hookLines(await readIfExists(join(env.HOME, ".zprofile"))),
      []
    );
    const hooked = spawnSync(
      "/bin/sh",
      ["-c", '. "$1"; printf %s "$TURBO_HOOKED"', "sh", dotfile],
      { encoding: "utf8", env: { PATH: "/usr/bin:/bin", HOME: env.HOME } }
    );
    assert.equal(hooked.stdout, "zsh", hooked.stderr);
  }));

for (const loginProfile of [".bash_profile", null]) {
  test(`adds the shell hook to bash profiles (${loginProfile ?? "no login profile"}) but never ~/.profile`, (t) =>
    withHarness(t, {}, async (_root, env) => {
      if (loginProfile) {
        await writeFile(join(env.HOME, loginProfile), "# login\n");
      }
      const result = runInstaller({ ...env, SHELL: "/bin/bash" });
      assert.equal(result.status, 0, result.stderr);
      const read = (name) => readIfExists(join(env.HOME, name));
      assert.equal(hookLines(await read(".bashrc")).length, 1);
      assert.match(hookLines(await read(".bashrc"))[0], / hook bash\)"/);
      if (loginProfile) {
        assert.equal(hookLines(await read(loginProfile)).length, 1);
      }
      assert.deepEqual(hookLines(await read(".profile")), []);
    }));
}

for (const scenario of [
  { name: "--no-shell-hook", args: ["--no-shell-hook"], path: true },
  { name: "TURBO_NO_SHELL_HOOK=1", env: { TURBO_NO_SHELL_HOOK: "1" }, path: true },
  { name: "an older release", hookSupport: false, path: true },
  { name: "--no-modify-path", args: ["--no-modify-path"], path: false },
  { name: "TURBO_NO_MODIFY_PATH=1", env: { TURBO_NO_MODIFY_PATH: "1" }, path: false },
  { name: "GitHub Actions", github: true, path: false }
]) {
  test(`does not add the shell hook for ${scenario.name}`, (t) =>
    withHarness(t, { hookSupport: scenario.hookSupport ?? true }, async (root, env) => {
      const githubPath = join(root, "github-path");
      if (scenario.github) {
        await writeFile(githubPath, "");
      }
      const result = runInstaller(
        {
          ...env,
          ...scenario.env,
          GITHUB_PATH: scenario.github ? githubPath : ""
        },
        scenario.args
      );
      assert.equal(result.status, 0, result.stderr);
      assert.doesNotMatch(result.stdout, /shell hook/);
      const zshrc = await readIfExists(join(env.HOME, ".zshrc"));
      assert.deepEqual(hookLines(zshrc), []);
      assert.equal(
        (zshrc ?? "").includes("Turborepo standalone installer PATH"),
        scenario.path
      );
    }));
}

test("prints the fish hook line instead of editing fish config", (t) =>
  withHarness(t, {}, async (_root, env) => {
    const fish = { ...env, SHELL: "/usr/bin/fish" };
    const result = runInstaller(fish);
    assert.equal(result.status, 0, result.stderr);
    assert.match(result.stdout, /config\.fish:\n {2}'.+\/turbo' hook fish \| source\n/);
    await assert.rejects(lstat(join(env.HOME, ".config")));
    const optedOut = runInstaller(fish, ["--no-shell-hook"]);
    assert.doesNotMatch(optedOut.stdout, /hook fish/);
  }));
