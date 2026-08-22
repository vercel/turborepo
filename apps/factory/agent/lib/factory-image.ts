/**
 * Single source of truth for the Turborepo factory image.
 *
 * The factory image is the sandbox base layer every Turborepo agent runs
 * on: a Turborepo checkout plus the complete toolchain `cargo build` and
 * `pnpm test` need. Both consumers of the image build it from the phases
 * declared here, so the Eve sandbox template and the snapshot published by
 * the merge webhook can never drift apart:
 *
 * - `agent/sandbox.ts` runs the phases sequentially in its Eve `bootstrap`.
 * - `workflows/factory-image.ts` writes them into one detached script and
 *   snapshots the sandbox when it finishes.
 *
 * Versions are pinned to the values the repository and CI already use
 * (`rust-toolchain.toml`, root `package.json`, `.github/actions/setup-*`);
 * `tests/factory-image.test.mjs` fails when they drift.
 */

import { createHash } from "node:crypto";

/**
 * Bump to force every factory image to rebuild even when no pinned
 * version changed (for example after fixing a provisioning script bug
 * that a version comparison cannot detect).
 */
export const FACTORY_IMAGE_VERSION = "1";

export interface FactoryPerformanceTool {
  /** Executable installed into `CARGO_HOME/bin`. */
  readonly binary: string;
  /** Crate installed with `cargo install --locked`. */
  readonly crate: string;
}

export interface FactoryImageSpec {
  /** Published checksum for the Cap'n Proto source fallback build. */
  readonly capnprotoSha256: string;
  readonly capnprotoVersion: string;
  /** Shared `CARGO_HOME`, readable and writable by every sandbox user. */
  readonly cargoHome: string;
  /** Canonical Turborepo checkout inside the image. */
  readonly checkoutPath: string;
  /** Symlinks pointing at {@link checkoutPath} for each consumer's cwd. */
  readonly linkPaths: readonly string[];
  readonly nodeMajor: number;
  readonly performanceTools: readonly FactoryPerformanceTool[];
  readonly pnpmVersion: string;
  readonly protocVersion: string;
  readonly repositoryUrl: string;
  readonly rustChannel: string;
  readonly rustComponents: readonly string[];
  readonly rustupHome: string;
  /** Build markers the merge webhook workflow polls. */
  readonly stateDirectory: string;
  readonly zigVersion: string;
}

export const FACTORY_IMAGE_SPEC: FactoryImageSpec = {
  capnprotoSha256:
    "07167580e563f5e821e3b2af1c238c16ec7181612650c5901330fa9a0da50939",
  capnprotoVersion: "1.1.0",
  cargoHome: "/usr/local/cargo",
  checkoutPath: "/factory/turborepo",
  linkPaths: ["/workspace/turborepo", "/vercel/sandbox/turborepo"],
  nodeMajor: 24,
  performanceTools: [
    { binary: "hyperfine", crate: "hyperfine" },
    { binary: "cargo-bloat", crate: "cargo-bloat" },
    { binary: "twiggy", crate: "twiggy" }
  ],
  pnpmVersion: "10.28.0",
  protocVersion: "26.1",
  repositoryUrl: "https://github.com/vercel/turborepo.git",
  rustChannel: "nightly-2026-05-22",
  rustComponents: ["rustfmt", "clippy"],
  rustupHome: "/usr/local/rustup",
  stateDirectory: "/factory/state",
  zigVersion: "0.15.2"
};

/** Vercel Sandbox image every factory sandbox starts from. */
export const FACTORY_IMAGE_BASE = "vercel/eve:latest";

/** Marker written to `state/phase` once every phase has run. */
export const FACTORY_IMAGE_DONE_PHASE = "done";

export interface FactoryImagePhase {
  readonly id: string;
  /** Shell body, appended to {@link factoryImagePreamble}. */
  readonly script: string;
  readonly title: string;
}

export interface FactoryImageOptions {
  /** Commit to check out. Must be a full or abbreviated commit SHA. */
  readonly revision: string;
  /**
   * Compile `turbo` once so the snapshot carries a warm `target/`
   * directory. Skipped for the Eve template, whose bootstrap runs inside
   * the deployment build.
   */
  readonly warmBuild?: boolean;
}

/** Ceiling for one `cargo install` of a performance tool. */
const TOOL_INSTALL_TIMEOUT_SECONDS = 900;
/** Ceiling for the optional warm `cargo build`. */
const WARM_BUILD_TIMEOUT_SECONDS = 2400;

/** A commit SHA, or `main` when no commit could be resolved up front. */
const REVISION_PATTERN = /^(?:main|[0-9a-f]{7,40})$/;
/** Placeholder revision used when fingerprinting, so the fingerprint
 * describes the toolchain rather than the commit. */
const FINGERPRINT_REVISION = "0".repeat(40);

export function assertFactoryRevision(revision: string): string {
  if (!REVISION_PATTERN.test(revision)) {
    throw new Error(
      `Invalid Turborepo revision: ${JSON.stringify(revision)}. Expected a commit SHA or "main".`
    );
  }
  return revision;
}

/**
 * Shell helpers shared by every phase. Sourced ahead of each phase in the
 * sequential runner and once at the top of the detached script, so a phase
 * body behaves identically either way.
 */
export function factoryImagePreamble(spec = FACTORY_IMAGE_SPEC): string {
  return `set -euo pipefail

FACTORY_STATE=${spec.stateDirectory}
FACTORY_REPO=${spec.checkoutPath}
export CARGO_HOME=${spec.cargoHome}
export RUSTUP_HOME=${spec.rustupHome}
export PATH=${spec.cargoHome}/bin:/usr/local/bin:$PATH
export DEBIAN_FRONTEND=noninteractive
export PLAYWRIGHT_SKIP_BROWSER_DOWNLOAD=1

factory_log() { printf '[factory-image] %s\\n' "$*"; }

factory_warn() {
  factory_log "WARNING: $*"
  mkdir -p "$FACTORY_STATE" 2>/dev/null || true
  printf '%s\\n' "$*" >> "$FACTORY_STATE/warnings" 2>/dev/null || true
}

factory_phase() {
  factory_log "phase: $1"
  mkdir -p "$FACTORY_STATE" 2>/dev/null || true
  printf '%s\\n' "$1" > "$FACTORY_STATE/phase" 2>/dev/null || true
}

have() { command -v "$1" >/dev/null 2>&1; }

# Bounds the phases that compile Rust. The Eve template provisions
# itself inside the deployment build, so one pathological crate must not
# be able to hold the whole build open.
factory_timeout() {
  seconds="$1"
  shift
  if have timeout; then
    timeout "$seconds" "$@"
  else
    "$@"
  fi
}

as_root() {
  if [ "$(id -u)" -eq 0 ]; then
    "$@"
  else
    sudo -n "$@"
  fi
}

pkg_install() {
  if have apt-get; then
    as_root apt-get update -y
    as_root apt-get install -y --no-install-recommends "$@"
  elif have dnf; then
    as_root dnf install -y "$@"
  elif have yum; then
    as_root yum install -y "$@"
  elif have apk; then
    as_root apk add --no-cache "$@"
  else
    factory_log "no supported package manager is available"
    return 1
  fi
}

factory_arch() {
  case "$(uname -m)" in
    x86_64 | amd64) printf 'x86_64\\n' ;;
    aarch64 | arm64) printf 'aarch64\\n' ;;
    *)
      printf 'unsupported architecture: %s\\n' "$(uname -m)" >&2
      return 1
      ;;
  esac
}

# Wraps a CARGO_HOME binary in /usr/local/bin so it works in non-login
# shells, which never source /etc/profile.d.
factory_wrap() {
  as_root tee "/usr/local/bin/$1" > /dev/null <<WRAPPER
#!/bin/sh
export CARGO_HOME=${spec.cargoHome}
export RUSTUP_HOME=${spec.rustupHome}
exec ${spec.cargoHome}/bin/$1 "\\$@"
WRAPPER
  as_root chmod 0755 "/usr/local/bin/$1"
}`;
}

function systemPackagesPhase(spec: FactoryImageSpec): string {
  return `if have apt-get; then
  pkg_install build-essential pkg-config lld libssl-dev jq zstd curl git \\
    unzip xz-utils ca-certificates capnproto libcapnp-dev
elif have dnf || have yum; then
  pkg_install gcc gcc-c++ make pkgconf-pkg-config lld openssl-devel jq \\
    zstd curl git unzip xz tar ca-certificates capnproto capnproto-devel
elif have apk; then
  pkg_install build-base pkgconf lld openssl-dev jq zstd curl git unzip xz \\
    tar ca-certificates capnproto capnproto-dev
else
  factory_log "no supported package manager is available"
  exit 1
fi

# Cap'n Proto is absent from some minimal images; fall back to the
# published source release, verified against its documented checksum.
if ! have capnp; then
  version=${spec.capnprotoVersion}
  archive="capnproto-c++-$version.tar.gz"
  curl --fail --show-error --silent --location --output "/tmp/$archive" \\
    "https://capnproto.org/$archive"
  printf '%s  %s\\n' ${spec.capnprotoSha256} "/tmp/$archive" \\
    | sha256sum --check --strict
  tar -zxf "/tmp/$archive" -C /tmp
  cd "/tmp/capnproto-c++-$version"
  ./configure --prefix=/usr --disable-shared
  make -j"$(nproc)"
  as_root make install
  cd /tmp
  rm -rf "/tmp/capnproto-c++-$version" "/tmp/$archive"
fi

# .cargo/config.toml links Linux builds with -fuse-ld=lld.
if ! have ld.lld; then
  if have lld; then
    as_root ln -sf "$(command -v lld)" /usr/local/bin/ld.lld
  else
    factory_log "lld is required to link Turborepo on Linux"
    exit 1
  fi
fi`;
}

function nodePhase(spec: FactoryImageSpec): string {
  return `current=0
if have node; then
  current="$(node -p 'process.versions.node.split(".")[0]')"
fi
if [ "$current" -lt ${spec.nodeMajor} ]; then
  factory_log "installing Node.js ${spec.nodeMajor} (found major $current)"
  if have apt-get; then
    curl --fail --show-error --silent --location \\
      "https://deb.nodesource.com/setup_${spec.nodeMajor}.x" \\
      | as_root bash -
    pkg_install nodejs
  elif have dnf || have yum; then
    pkg_install "nodejs${spec.nodeMajor}" || pkg_install nodejs
  else
    pkg_install nodejs npm
  fi
fi
node --version`;
}

function pnpmPhase(spec: FactoryImageSpec): string {
  return `if ! have pnpm || [ "$(pnpm --version)" != "${spec.pnpmVersion}" ]; then
  as_root npm install --force --global "pnpm@${spec.pnpmVersion}"
fi
pnpm --version`;
}

function rustPhase(spec: FactoryImageSpec): string {
  const components = spec.rustComponents
    .map((component) => `--component ${component}`)
    .join(" ");
  const wrapped = [
    "cargo",
    "rustc",
    "rustdoc",
    "rustup",
    "rustfmt",
    "cargo-fmt",
    "cargo-clippy",
    "clippy-driver"
  ];
  return `as_root mkdir -p ${spec.rustupHome} ${spec.cargoHome}
as_root chmod -R a+rwX ${spec.rustupHome} ${spec.cargoHome}

if ! [ -x ${spec.cargoHome}/bin/rustup ]; then
  curl --proto '=https' --tlsv1.2 --fail --show-error --silent \\
    https://sh.rustup.rs \\
    | sh -s -- --default-toolchain ${spec.rustChannel} --profile minimal \\
      ${components} --no-modify-path -y
fi

${spec.cargoHome}/bin/rustup toolchain install ${spec.rustChannel} \\
  --profile minimal ${components}
${spec.cargoHome}/bin/rustup default ${spec.rustChannel}
as_root chmod -R a+rwX ${spec.rustupHome} ${spec.cargoHome}

${wrapped.map((binary) => `factory_wrap ${binary}`).join("\n")}

as_root tee /etc/profile.d/10-factory-image.sh > /dev/null <<'PROFILE'
export CARGO_HOME=${spec.cargoHome}
export RUSTUP_HOME=${spec.rustupHome}
export ZIG_GLOBAL_CACHE_DIR=/usr/local/zig-cache
export FACTORY_REPO_PATH=${spec.checkoutPath}
export PATH="${spec.cargoHome}/bin:/usr/local/bin:$PATH"
PROFILE
as_root chmod 0644 /etc/profile.d/10-factory-image.sh

rustc --version`;
}

function protocPhase(spec: FactoryImageSpec): string {
  return `want=${spec.protocVersion}
if ! have protoc || [ "$(protoc --version | awk '{print $2}')" != "$want" ]; then
  case "$(factory_arch)" in
    x86_64) asset="x86_64" ;;
    aarch64) asset="aarch_64" ;;
  esac
  curl --fail --show-error --silent --location --output /tmp/protoc.zip \\
    "https://github.com/protocolbuffers/protobuf/releases/download/v$want/protoc-$want-linux-$asset.zip"
  as_root unzip -o -q /tmp/protoc.zip -d /usr/local 'bin/protoc' 'include/*'
  as_root chmod 0755 /usr/local/bin/protoc
  rm -f /tmp/protoc.zip
fi
protoc --version`;
}

function zigPhase(spec: FactoryImageSpec): string {
  return `want=${spec.zigVersion}
if ! have zig || [ "$(zig version)" != "$want" ]; then
  arch="$(factory_arch)"
  curl --fail --show-error --silent --location --output /tmp/zig.tar.xz \\
    "https://ziglang.org/download/$want/zig-$arch-linux-$want.tar.xz"
  as_root rm -rf /usr/local/zig
  as_root mkdir -p /usr/local/zig
  as_root tar -xJf /tmp/zig.tar.xz -C /usr/local/zig --strip-components 1
  as_root ln -sf /usr/local/zig/zig /usr/local/bin/zig
  rm -f /tmp/zig.tar.xz
fi
as_root mkdir -p /usr/local/zig-cache
as_root chmod -R a+rwX /usr/local/zig-cache
zig version`;
}

function checkoutPhase(
  spec: FactoryImageSpec,
  options: FactoryImageOptions
): string {
  const revision = assertFactoryRevision(options.revision);
  const links = spec.linkPaths
    .map(
      (link) => `as_root mkdir -p "$(dirname ${link})"
if [ -e ${link} ] && [ ! -L ${link} ]; then
  factory_log "replacing stale checkout at ${link}"
  as_root rm -rf ${link}
fi
as_root ln -sfn "$FACTORY_REPO" ${link}`
    )
    .join("\n");
  return `as_root mkdir -p "$(dirname "$FACTORY_REPO")" /workspace
as_root git config --system --add safe.directory "$FACTORY_REPO" || true

if [ ! -d "$FACTORY_REPO/.git" ]; then
  # Only on first checkout: a recursive chown over node_modules and
  # target/ would cost more than the fetch it precedes.
  as_root chown "$(id -u):$(id -g)" "$(dirname "$FACTORY_REPO")"
  git init --initial-branch=main "$FACTORY_REPO"
  git -C "$FACTORY_REPO" remote add origin ${spec.repositoryUrl}
fi

# Fetching the exact commit keeps the checkout shallow while still
# pinning the snapshot to the revision that triggered the build.
git -C "$FACTORY_REPO" fetch --depth=1 --force origin ${revision}
git -C "$FACTORY_REPO" reset --hard FETCH_HEAD
git -C "$FACTORY_REPO" clean -ffd

${links}

git -C "$FACTORY_REPO" rev-parse HEAD`;
}

function nodeModulesPhase(): string {
  return `cd "$FACTORY_REPO"
pnpm install --frozen-lockfile
test -d "$FACTORY_REPO/node_modules"`;
}

function cargoRegistryPhase(): string {
  return `cd "$FACTORY_REPO"
cargo fetch --locked`;
}

function performanceToolsPhase(spec: FactoryImageSpec): string {
  const installs = spec.performanceTools
    .map(
      (tool) => `if have ${tool.binary}; then
  factory_log "${tool.binary} is already installed"
elif (cd /tmp && factory_timeout ${TOOL_INSTALL_TIMEOUT_SECONDS} cargo install --locked ${tool.crate}); then
  factory_wrap ${tool.binary}
else
  factory_warn "could not install ${tool.crate}"
fi`
    )
    .join("\n");
  // The performance skill reaches for these; a broken upstream release
  // must not block publishing an otherwise complete image, so failures
  // are recorded as warnings instead of failing the build.
  return installs;
}

function warmBuildPhase(): string {
  return `cd "$FACTORY_REPO"
if ! factory_timeout ${WARM_BUILD_TIMEOUT_SECONDS} cargo build --package turbo; then
  factory_warn "warm cargo build failed; the snapshot has no warm target/"
fi`;
}

function verifyPhase(spec: FactoryImageSpec): string {
  const optional = spec.performanceTools
    .map(
      (tool) =>
        `have ${tool.binary} || factory_warn "${tool.binary} is missing"`
    )
    .join("\n");
  return `for tool in node pnpm cargo rustc protoc capnp zig jq zstd git \\
  ld.lld; do
  if ! have "$tool"; then
    factory_log "required tool is missing: $tool"
    exit 1
  fi
done

# rustc prints its semver, not the channel name, so assert the pinned
# toolchain through rustup and exercise it directly.
if ! rustup toolchain list | grep -q '^${spec.rustChannel}'; then
  factory_log "${spec.rustChannel} is not installed"
  rustup toolchain list
  exit 1
fi
rustup run ${spec.rustChannel} rustc --version
rustup run ${spec.rustChannel} cargo fmt --version > /dev/null
rustup run ${spec.rustChannel} cargo clippy --version > /dev/null
test -d "$FACTORY_REPO/crates"
test -d "$FACTORY_REPO/node_modules"

${optional}

mkdir -p "$FACTORY_STATE"
cat > "$FACTORY_STATE/image.json" <<JSON
{
  "capnp": "$(capnp --version | tr -d '"' | tail -n1)",
  "commit": "$(git -C "$FACTORY_REPO" rev-parse HEAD)",
  "node": "$(node --version)",
  "pnpm": "$(pnpm --version)",
  "protoc": "$(protoc --version)",
  "rustc": "$(rustc --version)",
  "zig": "$(zig version)"
}
JSON
cat "$FACTORY_STATE/image.json"`;
}

/**
 * Ordered provisioning phases for one factory image build.
 *
 * Every phase is idempotent: running the list against a sandbox that
 * already booted from a published snapshot re-checks each pinned version,
 * fast-forwards the checkout, and skips the work that is already done.
 */
export function factoryImagePhases(
  options: FactoryImageOptions,
  spec: FactoryImageSpec = FACTORY_IMAGE_SPEC
): FactoryImagePhase[] {
  const phases: FactoryImagePhase[] = [
    {
      id: "system-packages",
      script: systemPackagesPhase(spec),
      title: "Install build tooling, Cap'n Proto, and LLD"
    },
    { id: "node", script: nodePhase(spec), title: "Install Node.js" },
    { id: "pnpm", script: pnpmPhase(spec), title: "Install pnpm" },
    {
      id: "rust",
      script: rustPhase(spec),
      title: "Install the Rust toolchain"
    },
    { id: "protoc", script: protocPhase(spec), title: "Install protoc" },
    { id: "zig", script: zigPhase(spec), title: "Install Zig" },
    {
      id: "checkout",
      script: checkoutPhase(spec, options),
      title: "Check out Turborepo"
    },
    {
      id: "node-modules",
      script: nodeModulesPhase(),
      title: "Install workspace dependencies"
    },
    {
      id: "cargo-registry",
      script: cargoRegistryPhase(),
      title: "Warm the Cargo registry"
    },
    {
      id: "performance-tools",
      script: performanceToolsPhase(spec),
      title: "Install performance tooling"
    }
  ];
  if (options.warmBuild === true) {
    phases.push({
      id: "warm-build",
      script: warmBuildPhase(),
      title: "Compile turbo once"
    });
  }
  phases.push({
    id: "verify",
    script: verifyPhase(spec),
    title: "Verify the image"
  });
  return phases;
}

/**
 * One-shot script for the merge webhook workflow. It records the current
 * phase and the final exit code under the state directory so the workflow
 * can poll a detached build across step boundaries.
 */
export function factoryImageScript(
  options: FactoryImageOptions,
  spec: FactoryImageSpec = FACTORY_IMAGE_SPEC
): string {
  const phases = factoryImagePhases(options, spec)
    .map(
      (phase) => `factory_phase ${phase.id}
${phase.script}`
    )
    .join("\n\n");
  return `#!/usr/bin/env bash
${factoryImagePreamble(spec)}

as_root mkdir -p "$FACTORY_STATE"
as_root chown "$(id -u):$(id -g)" "$(dirname "$FACTORY_STATE")" "$FACTORY_STATE"
: > "$FACTORY_STATE/warnings"
rm -f "$FACTORY_STATE/exit"

on_exit() {
  code=$?
  printf '%s\\n' "$code" > "$FACTORY_STATE/exit"
  factory_log "finished with exit code $code"
}
trap on_exit EXIT

${phases}

factory_phase ${FACTORY_IMAGE_DONE_PHASE}`;
}

/** Provisioning script and log, relative to the sandbox working directory. */
export const FACTORY_IMAGE_SCRIPT_FILE = "factory-image-provision.sh";
export const FACTORY_IMAGE_LOG_FILE = "factory-image-provision.log";

const LOG_TAIL_LINES = 40;
const WARNINGS_MARKER = "--factory-warnings--";
const MANIFEST_MARKER = "--factory-manifest--";
const LOG_MARKER = "--factory-log--";

/**
 * Detaches the provisioning script so it survives the step that started
 * it. The workflow then polls {@link factoryImageProgressCommand} instead
 * of holding a function invocation open for the whole build.
 */
export function factoryImageStartCommand(): string {
  return `set -euo pipefail
setsid bash -lc 'bash ${FACTORY_IMAGE_SCRIPT_FILE}' \\
  > ${FACTORY_IMAGE_LOG_FILE} 2>&1 < /dev/null &
printf 'started\\n'`;
}

/** Reads every build marker in one round trip. */
export function factoryImageProgressCommand(
  spec: FactoryImageSpec = FACTORY_IMAGE_SPEC
): string {
  return `printf 'phase=%s\\n' "$(cat ${spec.stateDirectory}/phase 2>/dev/null || printf 'unknown')"
printf 'exit=%s\\n' "$(cat ${spec.stateDirectory}/exit 2>/dev/null || printf '-')"
printf '%s\\n' '${WARNINGS_MARKER}'
cat ${spec.stateDirectory}/warnings 2>/dev/null || true
printf '%s\\n' '${MANIFEST_MARKER}'
cat ${spec.stateDirectory}/image.json 2>/dev/null || true
printf '%s\\n' '${LOG_MARKER}'
tail -n ${LOG_TAIL_LINES} ${FACTORY_IMAGE_LOG_FILE} 2>/dev/null || true`;
}

export interface FactoryImageProgress {
  /** `null` while the script is still running. */
  readonly exitCode: number | null;
  readonly logTail: string;
  readonly manifest: Readonly<Record<string, string>> | null;
  readonly phase: string;
  readonly warnings: readonly string[];
}

export function parseFactoryImageProgress(
  stdout: string
): FactoryImageProgress {
  const lines = stdout.split("\n");
  const sections = new Map<string, string[]>([
    ["header", []],
    [WARNINGS_MARKER, []],
    [MANIFEST_MARKER, []],
    [LOG_MARKER, []]
  ]);
  let section = "header";
  for (const line of lines) {
    if (sections.has(line.trim()) && line.trim() !== "header") {
      section = line.trim();
      continue;
    }
    sections.get(section)?.push(line);
  }

  const header = sections.get("header") ?? [];
  const value = (key: string): string => {
    const match = header.find((line) => line.startsWith(`${key}=`));
    return match === undefined ? "" : match.slice(key.length + 1).trim();
  };
  const exit = value("exit");
  const exitCode = /^-?\d+$/.test(exit) ? Number(exit) : null;

  let manifest: Record<string, string> | null = null;
  const manifestText = (sections.get(MANIFEST_MARKER) ?? []).join("\n").trim();
  if (manifestText !== "") {
    try {
      const parsed: unknown = JSON.parse(manifestText);
      if (typeof parsed === "object" && parsed !== null) {
        manifest = Object.fromEntries(
          Object.entries(parsed as Record<string, unknown>).map(
            ([key, entry]) => [key, String(entry)]
          )
        );
      }
    } catch {
      manifest = null;
    }
  }

  return {
    exitCode,
    logTail: (sections.get(LOG_MARKER) ?? []).join("\n").trim(),
    manifest,
    phase: value("phase") || "unknown",
    warnings: (sections.get(WARNINGS_MARKER) ?? [])
      .map((line) => line.trim())
      .filter(Boolean)
  };
}

/**
 * Stable identity of the toolchain this module installs. Changing a
 * pinned version, a phase script, or {@link FACTORY_IMAGE_VERSION}
 * rotates the fingerprint, which rebuilds both the Eve sandbox template
 * and the published snapshot. The checked-out commit is deliberately
 * excluded: it changes on every merge and is fast-forwarded per session.
 */
export function factoryImageFingerprint(
  spec: FactoryImageSpec = FACTORY_IMAGE_SPEC
): string {
  const phases = factoryImagePhases(
    { revision: FINGERPRINT_REVISION, warmBuild: true },
    spec
  );
  const digest = createHash("sha256")
    .update(FACTORY_IMAGE_VERSION)
    .update(" ")
    .update(JSON.stringify(spec))
    .update(" ")
    .update(factoryImagePreamble(spec))
    .update(" ")
    .update(phases.map((phase) => `${phase.id} ${phase.script}`).join(" "))
    .digest("hex");
  return digest.slice(0, 16);
}

export interface FactoryCommandResult {
  readonly exitCode: number;
  readonly stderr: string;
  readonly stdout: string;
}

/**
 * Minimal command surface shared by an Eve `SandboxSession` and a
 * `@vercel/sandbox` `Sandbox`, so both consumers run the same phases.
 */
export interface FactoryCommandRunner {
  run(command: string): PromiseLike<FactoryCommandResult>;
}

export interface FactoryPhaseReport {
  readonly id: string;
  readonly title: string;
}

/**
 * Runs the phases one command at a time, failing on the first phase that
 * exits non-zero. Used by the Eve sandbox bootstrap, where per-phase
 * errors are far easier to read than one merged transcript.
 */
export async function runFactoryImagePhases(
  runner: FactoryCommandRunner,
  options: FactoryImageOptions,
  onPhase?: (phase: FactoryPhaseReport) => void,
  spec: FactoryImageSpec = FACTORY_IMAGE_SPEC
): Promise<void> {
  const preamble = factoryImagePreamble(spec);
  for (const phase of factoryImagePhases(options, spec)) {
    onPhase?.({ id: phase.id, title: phase.title });
    const result = await runner.run(
      `${preamble}\n\nfactory_phase ${phase.id}\n${phase.script}`
    );
    if (result.exitCode !== 0) {
      const output = [result.stdout, result.stderr]
        .map((stream) => stream.trim())
        .filter(Boolean)
        .join("\n");
      throw new Error(
        `Factory image phase "${phase.id}" (${phase.title}) failed with exit code ${result.exitCode}${output === "" ? "." : `:\n${output}`}`
      );
    }
  }
}
