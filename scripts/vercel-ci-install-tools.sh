#!/usr/bin/env bash
# Installs the tools Vercel CI tasks need but its images don't include.
#
# Vercel CI only gives tasks the files inside the repository, and puts
# `node_modules/.bin` on each task's PATH. So the tools go under
# `node_modules/.vercel-ci`, and small wrappers in `node_modules/.bin` expose them.
#
# - turbo: this repository has no turbo dependency, so CI installs the latest canary.
# - cargo and rustc: `futureFlags.experimentalCargoWorkspaces` makes every turbo
#   command run `cargo metadata`, so turbo can't start without them.
set -euo pipefail

root="$(pwd)"
tools="$root/node_modules/.vercel-ci"
bin="$root/node_modules/.bin"
export RUSTUP_HOME="$tools/rustup"
export CARGO_HOME="$tools/cargo"
mkdir -p "$tools" "$bin"

npm install --prefix "$tools/turbo" --no-save --no-audit --no-fund --no-package-lock turbo@canary

curl --proto '=https' --tlsv1.2 -fsSL https://sh.rustup.rs |
  sh -s -- -y --no-modify-path --profile minimal --default-toolchain none
# Installs the toolchain that rust-toolchain.toml pins.
"$CARGO_HOME/bin/rustup" toolchain install

# Each wrapper finds the tools relative to its own path, so the wrappers still
# work when the repository is restored somewhere else.
write_wrapper() {
  local name=$1 target=$2
  cat > "$bin/$name" <<EOF
#!/usr/bin/env bash
tools="\$(cd "\$(dirname "\${BASH_SOURCE[0]}")/../.vercel-ci" && pwd)"
export RUSTUP_HOME="\$tools/rustup"
export CARGO_HOME="\$tools/cargo"
exec "\$tools/$target" "\$@"
EOF
  chmod +x "$bin/$name"
}
write_wrapper turbo turbo/node_modules/.bin/turbo
write_wrapper cargo cargo/bin/cargo
write_wrapper rustc cargo/bin/rustc

# Download the crates `cargo metadata` reads now, so tasks don't fetch them.
"$bin/cargo" fetch --locked
"$bin/turbo" --version
"$bin/cargo" --version
