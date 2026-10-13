#!/bin/sh
# Managed setup journey for an installed standalone turbo, shared by the POSIX
# release smokes. One repository uses Node.js and pnpm, Rust, uv and Python,
# Go, and a root-declared additional tool. turbo's PATH holds only `mkdir` and
# `cp` (no Node.js, JS package manager, Rust, uv, Python, or Go), and its proxy
# is unreachable; only the loopback fixture mirror answers, through the
# TURBO_SETUP_* mirror variables. The journey checks that:
#
# - `turbo setup` from a subdirectory creates turbo.lock, provisions every
#   tool, and installs locked dependencies with each native frozen installer;
# - a frozen CI clone refuses tampered mirror bytes and installs nothing;
# - plain `turbo run` uses the managed tools for turbo's own probes and for
#   every task, in strict and loose modes;
# - repeats, CI, offline setup, and `--check` make no requests;
# - the bash and zsh hooks the installer added activate the tools inside the
#   repository and restore PATH exactly outside it;
# - `turbo exec` runs commands with the managed tools and never installs;
# - setup repairs a damaged installation that run and exec refuse.
#
# Usage: scripts/standalone-setup-journey.sh <turbo> <node-for-the-mirror> <profiles>
# <profiles> holds the .bashrc and .zshrc that the installer wrote for <turbo>.
set -eu
umask 022

fail() {
  printf 'standalone setup journey: %s\n' "$1" >&2
  exit 1
}

# Miette may wrap diagnostics across lines with `|` gutters.
contains() {
  case "$(printf '%s' "$1" | tr -d ' |\n')" in
    *"$(printf '%s' "$2" | tr -d ' |\n')"*) return 0 ;;
  esac
  return 1
}

[ "$#" -eq 3 ] || fail 'usage: standalone-setup-journey.sh <turbo> <node-for-the-mirror> <profiles>'
turbo=$1
profiles=$3
scripts=$(cd "$(dirname "$0")" && pwd)
case "$(uname -s):$(uname -m)" in
  Linux:x86_64) arch=x64 triple=x86_64-unknown-linux-gnu ;;
  Linux:aarch64) arch=arm64 triple=aarch64-unknown-linux-gnu ;;
  Darwin:x86_64) arch=x64 triple=x86_64-apple-darwin ;;
  Darwin:arm64) arch=arm64 triple=aarch64-apple-darwin ;;
  *) fail "unsupported journey host: $(uname -s)/$(uname -m)" ;;
esac
bash=$(command -v bash) || fail 'the shell hook journey needs bash'
zsh=$(command -v zsh) || fail 'the shell hook journey needs zsh'
work=$(mktemp -d "${TMPDIR:-/tmp}/turbo-setup-journey.XXXXXX")
work=$(cd "$work" && pwd -P)
mirror=$work/mirror
mkdir "$mirror" "$work/home" "$work/clone-home" "$work/tmp" "$work/core"
# The stand-in rustup lays out its toolchain with these.
for utility in mkdir cp; do
  ln -s "$(command -v "$utility")" "$work/core/$utility"
done
"$2" "$scripts/setup-fixture-mirror.mjs" "$mirror" &
pid=$!
# Waiting reaps the mirror, so shells do not report it as terminated.
trap 'kill "$pid" 2>/dev/null || true; wait "$pid" 2>/dev/null || true; rm -rf "$work"' 0
tries=0
until [ -s "$mirror/ready" ]; do
  kill -0 "$pid" 2>/dev/null || fail 'the fixture mirror exited'
  tries=$((tries + 1))
  [ "$tries" -le 600 ] || fail 'the fixture mirror did not start'
  sleep 0.1
done
read -r port version <"$mirror/ready"

repo=$work/repo
web=$repo/apps/web
mkdir -p "$web" "$repo/crates/rs-app/src" "$repo/packages/py-app/src/py_app" "$repo/goapp"
flags='"experimentalSetup":true,"experimentalCargoWorkspaces":true,"experimentalPythonWorkspaces":true,"experimentalGoWorkspaces":true'
printf '{"futureFlags":{%s},"agentGuidance":false,"setup":{"additionalTools":%s},"tasks":{"build":{"cache":false}}}\n' \
  "$flags" "$(cat "$mirror/additional-tools.json")" >"$repo/turbo.json"
printf '/.turbo/\nnode_modules/\n/target/\n.venv/\n' >"$repo/.gitignore"
# Node.js and pnpm
printf '%s\n' "$version" >"$repo/.nvmrc"
printf '%s\n' '{"name":"journey","private":true,"packageManager":"pnpm@10.0.0"}' >"$repo/package.json"
printf "packages: ['apps/*']\n" >"$repo/pnpm-workspace.yaml"
printf "lockfileVersion: '9.0'\nimporters:\n  .: {}\n  apps/web: {}\n" >"$repo/pnpm-lock.yaml"
printf '%s\n' '{"name":"web","scripts":{"build":"smoke-tool && node probe.cjs"}}' >"$web/package.json"
cat >"$web/probe.cjs" <<'EOF'
const first = process.env.PATH.split(require("node:path").delimiter)[0];
const node = require("node:fs").realpathSync(process.execPath);
const secret = process.env.SMOKE_SECRET ?? "filtered";
console.log(`managed-probe ${node}|${process.version}|${process.arch}|${first}|${secret}`);
EOF
# Rust
printf '[workspace]\nmembers = ["crates/rs-app"]\nresolver = "2"\n\n[workspace.metadata]\nname = "rs-root"\n' >"$repo/Cargo.toml"
printf '[toolchain]\nchannel = "stable"\nprofile = "minimal"\n' >"$repo/rust-toolchain.toml"
printf 'version = 4\n\n[[package]]\nname = "rs-app"\nversion = "0.1.0"\n' >"$repo/Cargo.lock"
printf '[package]\nname = "rs-app"\nversion = "0.1.0"\nedition = "2021"\n' >"$repo/crates/rs-app/Cargo.toml"
: >"$repo/crates/rs-app/src/lib.rs"
# uv and Python
printf '[tool.turbo]\nname = "py-root"\n\n[tool.uv]\nrequired-version = ">=0.13"\n\n[tool.uv.workspace]\nmembers = ["packages/*"]\n' >"$repo/pyproject.toml"
printf '3.12\n' >"$repo/.python-version"
printf '[project]\nname = "py-app"\nversion = "0.1.0"\nrequires-python = ">=3.12"\ndependencies = []\n\n[build-system]\nrequires = ["uv_build>=0.8,<2"]\nbuild-backend = "uv_build"\n' \
  >"$repo/packages/py-app/pyproject.toml"
: >"$repo/packages/py-app/src/py_app/__init__.py"
printf 'version = 1\nrevision = 3\nrequires-python = ">=3.12"\n\n[manifest]\nmembers = ["py-app"]\n\n[[package]]\nname = "py-app"\nversion = "0.1.0"\nsource = { editable = "packages/py-app" }\n' \
  >"$repo/uv.lock"
# Go
printf 'go 1.22\n\nuse ./goapp\n' >"$repo/go.work"
printf 'module example.com/goapp\n\ngo 1.22\n' >"$repo/goapp/go.mod"
printf 'package main\n\nfunc main() {}\n' >"$repo/goapp/main.go"

base=https://127.0.0.1:$port
closed=http://127.0.0.1:9
home=$work/home ci=
# Sets $output and $code, and $requests to the mirror requests it made.
run() {
  dir=$1
  shift
  before=$(wc -l <"$mirror/requests.log")
  code=0
  output=$(cd "$dir" && env -i PATH="$work/core" HOME="$home" TMPDIR="$work/tmp" \
    TURBO_TELEMETRY_DISABLED=1 DO_NOT_TRACK=1 TURBO_NO_UPDATE_NOTIFIER=1 \
    HTTP_PROXY="$closed" HTTPS_PROXY="$closed" ALL_PROXY="$closed" NO_PROXY=127.0.0.1 \
    SSL_CERT_FILE="$scripts/fixtures/setup-mirror/ca.pem" \
    TURBO_SETUP_NODE_MIRROR="$base/node" TURBO_SETUP_NPM_REGISTRY="$base/npm" \
    TURBO_SETUP_RUST_MIRROR="$base/rust" TURBO_SETUP_GO_MIRROR="$base/golang" \
    TURBO_SETUP_UV_MIRROR="$base/astral" TURBO_SETUP_PYTHON_MIRROR="$base/astral" \
    TURBO_SETUP_ARTIFACT_MIRROR="$base/artifacts" \
    SMOKE_SECRET=present ${ci:+CI=1} "$turbo" "$@" 2>&1) || code=$?
  requests=$(($(wc -l <"$mirror/requests.log") - before))
}
succeeds() {
  run "$@"
  [ "$code" -eq 0 ] || fail "turbo $* exited $code: $output"
}
offline() {
  succeeds "$@"
  [ "$requests" -eq 0 ] || fail "turbo $* made $requests mirror requests: $output"
}
# Fails unless the stand-in tools logged each argument since the last check.
called() {
  log=$(cat "$mirror/calls.log")
  : >"$mirror/calls.log"
  for call in "$@"; do
    contains "$log" "$call" || fail "no managed \"$call\" call: $log"
  done
}
# Each native frozen installer ran with its managed tool, plus any other
# expected calls.
installs_dependencies() {
  for line in 'javascript dependencies: installed; pnpm-lock.yaml unchanged' \
    'python dependencies: installed; uv.lock unchanged' \
    'rust dependencies: installed; Cargo.lock unchanged' \
    'go dependencies: installed; go.sum unchanged'; do
    contains "$output" "$line" || fail "setup did not report \"$line\": $output"
  done
  called 'pnpm install --frozen-lockfile' 'uv sync --locked --all-packages' 'cargo fetch --locked' \
    'go mod download' "$@"
}

# First setup, from a subdirectory, through the mirror only: every tool, then
# every ecosystem's locked dependencies.
succeeds "$web" setup
contains "$output" 'Commit turbo.lock' || fail "first setup did not ask for a commit: $output"
[ "$requests" -gt 0 ] || fail 'first setup made no mirror requests'
installs_dependencies 'rustup toolchain install 1.99.0'
! grep -q 127.0.0.1 "$repo/turbo.lock" || fail 'turbo.lock records the mirror location'
for tool in node pnpm rustup rust uv python go smoke-tool; do
  grep -q "\"$tool\": *{" "$repo/turbo.lock" || fail "turbo.lock does not lock $tool: $(cat "$repo/turbo.lock")"
done
cp "$repo/turbo.lock" "$work/turbo.lock"
cp "$repo/.turbo/tools/manifest.json" "$work/manifest.json"
unchanged() {
  cmp -s "$repo/turbo.lock" "$work/turbo.lock" &&
    cmp -s "$repo/.turbo/tools/manifest.json" "$work/manifest.json" ||
    fail "turbo $1 changed turbo.lock or the installation"
}

# A clone with the committed lock and an empty download cache refuses tampered
# mirror bytes in frozen CI setup and installs nothing.
cp -R "$repo" "$work/clone"
rm -rf "$work/clone/.turbo"
touch "$mirror/tamper"
home=$work/clone-home ci=1
run "$work/clone" setup --tools-only
home=$work/home ci=
rm "$mirror/tamper"
[ "$code" -ne 0 ] && contains "$output" 'does not match the trusted digest' ||
  fail "frozen setup accepted a tampered artifact: $output"
[ ! -e "$work/clone/.turbo/tools/manifest.json" ] || fail 'a tampered artifact was installed'
cmp -s "$work/clone/turbo.lock" "$work/turbo.lock" || fail 'frozen setup changed turbo.lock'
: >"$mirror/calls.log"

# Plain turbo run: every task uses its managed tools (from apps/web, turbo
# infers only that package). The JavaScript task's node is managed Node for
# this architecture, first on PATH, and strict mode filters the caller's
# variables while loose mode passes them.
prints() {
  contains "$output" "$1" || fail "turbo run --env-mode=$mode did not print \"$1\": $output"
}
build() {
  mode=$2 secret=$3
  offline "$1" run build "--env-mode=$mode"
  prints 'fixture-pnpm 10.0.0 run build'
  prints "smoke-tool 1.2.3 secret=$secret"
  if [ "$1" = "$repo" ]; then
    prints "managed-cargo build --package=rs-app --locked toolchain=1.99.0-$triple secret=$secret"
    prints "managed-uv build --package=py-app python=cpython-3.12.12-"
    prints "downloads=never secret=$secret"
    prints "managed-go build ./... GOTOOLCHAIN=local goroot=locked secret=$secret"
  fi
  probe=$(printf '%s\n' "$output" | sed -n 's/.*managed-probe //p' | head -n 1)
  case "$probe" in
    "$repo/.turbo/tools/"*"|v$version|$arch|$repo/.turbo/tools/"*"|$secret") ;;
    *) fail "unexpected $mode task environment: ${probe:-$output}" ;;
  esac
  unchanged run
}
for dir in "$repo" "$web"; do
  build "$dir" strict filtered
  build "$dir" loose present
done
# turbo's own Cargo, uv, and Go discovery used the managed tools, too.
called 'cargo metadata' 'rustc -vV' 'uv --version' 'uv python find' 'go work edit -json' 'go env -json'
bin=$(printf '%s\n' "$probe" | cut -d '|' -f 4)

# Repeats reuse the tools without network and rerun the locked installers;
# frozen CI does the same. Offline setup prepares tools only, and offline
# `--check` reports readiness.
offline "$repo" setup
installs_dependencies
ci=1
offline "$repo" setup
ci=
installs_dependencies
offline "$repo" setup --tools-only --offline
offline "$repo" setup --check --offline
unchanged setup

# The installer's bash and zsh hooks activate exactly the recorded tools in
# the repository and its subdirectories, and leaving restores PATH exactly.
cp "$profiles/.bashrc" "$profiles/.zshrc" "$home/"
cat >"$work/hook-input" <<EOF
echo "start=\$PATH"
cd '$web'
echo "inside=\$PATH"
echo "node=\$(node -p process.version)"
echo "tool=\$(smoke-tool)"
cd '$work'
echo "outside=\$PATH"
exit
EOF
for shell in "$bash" "$zsh"; do
  before=$(wc -l <"$mirror/requests.log")
  hook=$(cd "$work" && env -i PATH="$work/core" HOME="$home" TMPDIR="$work/tmp" TERM=dumb \
    TURBO_TELEMETRY_DISABLED=1 DO_NOT_TRACK=1 TURBO_NO_UPDATE_NOTIFIER=1 \
    HTTP_PROXY="$closed" HTTPS_PROXY="$closed" ALL_PROXY="$closed" \
    "$shell" -i <"$work/hook-input" 2>"$work/hook-stderr") ||
    fail "$shell exited $?: $(cat "$work/hook-stderr")"
  value() {
    printf '%s\n' "$hook" | sed -n "s/^$1=//p" | head -n 1
  }
  start=$(value start)
  [ "$start" = "${turbo%/*}:$work/core" ] ||
    fail "$shell did not start with the installer's PATH: $hook $(cat "$work/hook-stderr")"
  [ "$(value inside)" = "$bin:$start" ] && [ "$(value outside)" = "$start" ] &&
    [ "$(value node)" = "v$version" ] && [ "$(value tool)" = 'smoke-tool 1.2.3 secret=filtered' ] ||
    fail "the $shell hook did not activate and restore the managed tools: $hook $(cat "$work/hook-stderr")"
  [ "$(wc -l <"$mirror/requests.log")" -eq "$before" ] || fail "the $shell hook made mirror requests"
done
unchanged hook

# turbo exec: managed node from the caller's directory with exact arguments,
# stdin, and exit codes, and every other managed tool; a clone without an
# installation fails with guidance and installs nothing.
exec_probe='console.log(["exec-probe", require("node:fs").realpathSync(process.execPath), process.cwd(), ...process.argv.slice(1)].join("|"))'
offline "$web" exec -- node -e "$exec_probe" 'a b' '' '$HOME' 'back\slash\' "q'uote"
line=$(printf '%s\n' "$output" | grep '^exec-probe|' | head -n 1)
case "$line" in
  "exec-probe|$repo/.turbo/tools/"*) ;;
  *) fail "turbo exec did not run managed node: $output" ;;
esac
[ "${line#*|*|}" = "$web|a b||\$HOME|back\\slash\\|q'uote" ] ||
  fail "turbo exec changed the directory or arguments: $line"
printf 'piped stdin' >"$work/stdin"
offline "$web" exec -- node -e 'process.stdin.pipe(process.stdout)' <"$work/stdin"
contains "$output" 'piped stdin' || fail "turbo exec did not forward stdin: $output"
run "$web" exec -- node -e 'process.exit(7)'
[ "$code" -eq 7 ] || fail "turbo exec exited $code, expected 7: $output"
runs() {
  expected=$1
  shift
  offline "$web" exec -- "$@"
  contains "$output" "$expected" || fail "turbo exec -- $* did not run the managed tool: $output"
}
runs 'fixture-pnpm 10.0.0 run build' pnpm run build
contains "$output" "|v$version|$arch|" || fail "turbo exec ran pnpm without managed node: $output"
runs "toolchain=1.99.0-$triple" cargo build
runs 'uv 0.13.0' uv --version
runs 'Python 3.12.12' python3 --version
runs 'go version go1.22.12' go version
runs 'smoke-tool 1.2.3 secret=present' smoke-tool
unchanged exec
run "$work/clone" exec -- node -e 'console.log("exec-ran")'
[ "$code" -ne 0 ] && [ "$requests" -eq 0 ] && contains "$output" 'turbo setup' &&
  ! contains "$output" 'exec-ran' || fail "turbo exec ran without an installation: $output"
[ ! -e "$work/clone/.turbo/tools/manifest.json" ] && cmp -s "$work/clone/turbo.lock" "$work/turbo.lock" ||
  fail 'turbo exec installed tools or changed turbo.lock'

# Damage: run and exec refuse before running anything, --check fails, and
# setup repairs it.
rm "$bin/pnpm"
run "$repo" run build
[ "$code" -ne 0 ] && contains "$output" 'turbo setup' || fail "run accepted a damaged installation: $output"
! contains "$output" 'managed-probe' || fail "a task ran on a damaged installation: $output"
run "$repo" exec -- node -e 'console.log("exec-ran")'
[ "$code" -ne 0 ] && [ "$requests" -eq 0 ] && contains "$output" 'turbo setup' &&
  ! contains "$output" 'exec-ran' || fail "exec accepted a damaged installation: $output"
run "$repo" setup --check --tools-only --offline
[ "$code" -ne 0 ] || fail "setup --check accepted a damaged installation: $output"
succeeds "$repo" setup --tools-only
cmp -s "$repo/turbo.lock" "$work/turbo.lock" || fail 'repair changed turbo.lock'
cp "$repo/.turbo/tools/manifest.json" "$work/manifest.json"
build "$repo" strict filtered

printf 'Standalone setup journey passed (%s): Node.js %s, pnpm 10.0.0, Rust 1.99.0, uv 0.13.0, Python 3.12.12, Go 1.22.12, smoke-tool 1.2.3, locked dependencies, and the bash and zsh hooks.\n' \
  "$arch" "$version"
