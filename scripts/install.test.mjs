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
  const binaryContents = `#!/bin/sh\nprintf '%s\\n' "$*" >> "$TURBO_BINARY_LOG"\n${
    hookSupport
      ? '[ "$1" = hook ] || exit 1\nprintf "TURBO_HOOKED=%s\\n" "$2"\n'
      : "exit 1\n"
  }`;
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
      TURBO_BINARY_LOG: join(root, "turbo.log"),
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
    assert.match(
      result.stderr,
      /Uninstall it using the tool that installed it/
    );
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

for (const shell of ["/bin/zsh", "/bin/bash", "/usr/bin/fish"]) {
  for (const hookSupport of [true, false]) {
    test(`install and upgrade do not activate shell hooks for ${shell} (hook support: ${hookSupport})`, (t) =>
      withHarness(t, { hookSupport }, async (_root, env) => {
        const shellEnv = { ...env, SHELL: shell };
        for (let run = 0; run < 2; run++) {
          const result = runInstaller(shellEnv);
          assert.equal(result.status, 0, result.stderr);
          assert.doesNotMatch(result.stdout, /shell hook|hook fish/);
          for (const contents of Object.values(
            await snapshotProfiles(env.HOME)
          )) {
            assert.deepEqual(hookLines(contents?.toString()), []);
          }
          await assert.rejects(readFile(env.TURBO_BINARY_LOG));
        }
        if (shell.endsWith("fish")) {
          assert.deepEqual(await readdir(env.HOME), []);
        } else {
          const profile = shell.endsWith("zsh") ? ".zshrc" : ".bashrc";
          assert.match(
            await readFile(join(env.HOME, profile), "utf8"),
            /Turborepo standalone installer PATH/
          );
        }
      }));
  }
}

for (const shell of ["zsh", "bash"]) {
  test(`upgrades leave existing ${shell} hooks untouched through symlinked profiles`, (t) =>
    withHarness(t, {}, async (root, env) => {
      const dotfile = join(root, "dotfile");
      const profile = join(env.HOME, shell === "zsh" ? ".zshrc" : ".bashrc");
      const marked = `eval "$('/old/turbo' hook ${shell})" ${HOOK_MARKER}`;
      const manual = `eval "$(turbo hook ${shell})"`;
      const original = `# mine\n${marked}\n${manual}\n${marked}\n`;
      await writeFile(dotfile, original);
      await symlink(dotfile, profile);
      for (let run = 0; run < 2; run++) {
        const result = runInstaller({ ...env, SHELL: `/bin/${shell}` });
        assert.equal(result.status, 0, result.stderr);
        const contents = await readFile(dotfile, "utf8");
        assert.ok(contents.startsWith(original));
        assert.deepEqual(hookLines(contents), [marked, marked]);
        assert.ok((await lstat(profile)).isSymbolicLink());
        await assert.rejects(readFile(env.TURBO_BINARY_LOG));
      }
    }));
}

for (const scenario of [
  { name: "--no-modify-path", args: ["--no-modify-path"] },
  { name: "TURBO_NO_MODIFY_PATH=1", env: { TURBO_NO_MODIFY_PATH: "1" } },
  { name: "GitHub Actions", github: true }
]) {
  test(`does not edit shell profiles for ${scenario.name}`, (t) =>
    withHarness(t, {}, async (root, env) => {
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
      assert.deepEqual(await readdir(env.HOME), []);
      await assert.rejects(readFile(env.TURBO_BINARY_LOG));
    }));
}

const PROFILES = [
  ".profile",
  ".bash_profile",
  ".bash_login",
  ".bashrc",
  ".zprofile",
  ".zshrc",
  ".config/fish/config.fish"
];

async function snapshotProfiles(home) {
  const files = {};
  for (const name of PROFILES) {
    files[name] = await readFile(join(home, name)).catch(() => null);
  }
  return files;
}

function shellQuote(value) {
  return `'${value.replaceAll("'", "'\\''")}'`;
}

for (const scenario of [
  {
    name: "zsh profiles through a symlinked dotfile",
    shell: "/bin/zsh",
    profiles: { ".zshrc": "# mine\nexport EDITOR=vi\n" },
    symlink: ".zshrc",
    uninstall: { args: ["--uninstall"] }
  },
  {
    name: "bash profiles without a final newline or with a trailing blank line",
    shell: "/bin/bash",
    profiles: {
      ".bash_profile": "# login without a final newline",
      ".bashrc": "alias ll='ls -l'\n\n"
    },
    uninstall: { env: { TURBO_UNINSTALL: "1" } }
  },
  {
    name: "bash profiles in a fresh home",
    shell: "/bin/bash",
    profiles: {},
    uninstall: { args: ["--uninstall"] }
  },
  {
    name: "fish config",
    shell: "/usr/bin/fish",
    profiles: { ".config/fish/config.fish": "set -g fish_greeting\n" },
    uninstall: { args: ["--uninstall"] }
  }
]) {
  test(`uninstall restores ${scenario.name} byte for byte`, (t) =>
    withHarness(t, {}, async (root, env) => {
      const shellEnv = { ...env, SHELL: scenario.shell };
      for (const [name, contents] of Object.entries(scenario.profiles)) {
        const path = join(env.HOME, name);
        await mkdir(join(path, ".."), { recursive: true });
        if (name === scenario.symlink) {
          await writeFile(join(root, "dotfile"), contents);
          await symlink(join(root, "dotfile"), path);
        } else {
          await writeFile(path, contents);
        }
      }
      const before = await snapshotProfiles(env.HOME);
      const installed = runInstaller(shellEnv);
      assert.equal(installed.status, 0, installed.stderr);
      const afterInstall = await snapshotProfiles(env.HOME);
      if (scenario.shell.endsWith("fish")) {
        assert.deepEqual(afterInstall, before);
      } else {
        assert.notDeepEqual(afterInstall, before);
      }
      const requests = await readFile(env.TURBO_CURL_LOG, "utf8");

      const result = runInstaller(
        { ...shellEnv, ...scenario.uninstall.env },
        scenario.uninstall.args
      );
      assert.equal(result.status, 0, result.stderr);
      assert.deepEqual(await snapshotProfiles(env.HOME), before);
      if (scenario.symlink) {
        assert.ok(
          (await lstat(join(env.HOME, scenario.symlink))).isSymbolicLink()
        );
      }
      const binary = join(env.TURBO_INSTALL_DIR, "turbo");
      await assert.rejects(lstat(binary));
      assert.ok(result.stdout.includes(`Removed ${binary}\n`), result.stdout);
      if (!scenario.shell.endsWith("fish")) {
        assert.match(result.stdout, /Removed the turbo PATH entry from /);
      }
      assert.doesNotMatch(result.stdout, /Left /);
      assert.equal(await readFile(env.TURBO_CURL_LOG, "utf8"), requests);
    }));
}

for (const shell of ["zsh", "bash"]) {
  test(`uninstall still removes legacy installer-owned ${shell} hooks`, (t) =>
    withHarness(t, {}, async (root, env) => {
      const destination = join(env.TURBO_INSTALL_DIR, "turbo");
      const manual = `eval "$(turbo hook ${shell})"\n`;
      const legacy = `eval "$(${shellQuote(destination)} hook ${shell})" ${HOOK_MARKER}`;
      const dotfile = join(root, "dotfile");
      const profile = join(env.HOME, shell === "zsh" ? ".zshrc" : ".bashrc");
      await writeFile(dotfile, `${manual}\n${legacy}\n`);
      await symlink(dotfile, profile);

      const result = runInstaller(env, ["--uninstall"]);
      assert.equal(result.status, 0, result.stderr);
      assert.match(result.stdout, /Removed the turbo shell hook from /);
      assert.equal(await readFile(dotfile, "utf8"), manual);
      assert.ok((await lstat(profile)).isSymbolicLink());
      await assert.rejects(readFile(env.TURBO_CURL_LOG));
      await assert.rejects(readFile(env.TURBO_BINARY_LOG));
    }));
}

test("uninstall leaves binaries and profile lines the installer does not own", (t) =>
  withHarness(t, {}, async (root, env) => {
    const destination = join(env.TURBO_INSTALL_DIR, "turbo");
    const linkTarget = join(root, "linked-turbo");
    await writeFile(linkTarget, "keep me");
    await mkdir(env.TURBO_INSTALL_DIR, { recursive: true });
    await symlink(linkTarget, destination);
    const otherDirectory = join(root, "other-bin");
    const otherTurbo = join(otherDirectory, "turbo");
    const ran = join(root, "other-turbo-ran");
    await mkdir(otherDirectory);
    await writeFile(otherTurbo, `#!/bin/sh\nprintf ran > '${ran}'\n`);
    await chmod(otherTurbo, 0o755);

    const zshrc = [
      `eval "$(${shellQuote(destination)} hook zsh)"`,
      `eval "$('/elsewhere/turbo' hook zsh)" ${HOOK_MARKER}`,
      `# Turborepo standalone installer PATH: ${env.TURBO_INSTALL_DIR}`,
      'export PATH="/edited:$PATH"',
      ""
    ].join("\n");
    await writeFile(join(env.HOME, ".zshrc"), zshrc);
    const fishConfig = join(env.HOME, ".config", "fish", "config.fish");
    const fishLine = `${shellQuote(destination)} hook fish | source\n`;
    await mkdir(join(env.HOME, ".config", "fish"), { recursive: true });
    await writeFile(fishConfig, fishLine);

    const result = runInstaller(
      { ...env, PATH: `${otherDirectory}:${env.PATH}` },
      ["--uninstall"]
    );
    assert.equal(result.status, 0, result.stderr);
    assert.ok((await lstat(destination)).isSymbolicLink());
    assert.equal(await readFile(linkTarget, "utf8"), "keep me");
    await assert.rejects(readFile(ran));
    assert.equal(await readFile(join(env.HOME, ".zshrc"), "utf8"), zshrc);
    assert.equal(await readFile(fishConfig, "utf8"), fishLine);
    for (const message of [
      `Left ${destination} because it is not a regular file.`,
      `Left ${otherTurbo} because the installer did not install it.`,
      `Left lines in ${join(env.HOME, ".zshrc")} that are marked by the installer`,
      `Left the turbo hook line in ${fishConfig} because the installer did not add it`
    ]) {
      assert.ok(result.stdout.includes(message), result.stdout);
    }
    assert.doesNotMatch(result.stdout, /Removed/);
    await assert.rejects(readFile(env.TURBO_CURL_LOG));
  }));
