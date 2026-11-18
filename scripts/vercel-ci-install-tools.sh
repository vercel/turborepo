#!/usr/bin/env bash
# Vercel CI install command. This repository has no turbo dependency, so CI
# installs the latest turbo canary, then `turbo setup` installs the Rust
# toolchain and the dependencies. `futureFlags.experimentalCargoWorkspaces`
# makes every turbo command run `cargo metadata`, so turbo needs that toolchain.
#
# Vercel CI snapshots the whole drive that holds the repository, so both
# directories below reach every task cell:
# - `.vercel-ci-runtime` is Vercel CI's runtime prefix. Its `node_modules/.bin`
#   is on the PATH of the planning step and of every task.
# - `.turbo-tools` is the turbo tool store, which must be outside the repository.
set -euo pipefail

drive="$(dirname "$(pwd)")"
runtime="$drive/.vercel-ci-runtime"
export TURBO_TOOL_STORE="$drive/.turbo-tools"

npm install --prefix "$runtime" --no-audit --no-fund turbo@canary

# Replace npm's turbo link with a wrapper, so every turbo command finds the
# tool store without a TURBO_TOOL_STORE entry for each task.
turbo="$runtime/node_modules/.bin/turbo"
rm -f "$turbo"
cat > "$turbo" <<EOF
#!/usr/bin/env bash
export TURBO_TOOL_STORE="$TURBO_TOOL_STORE"
exec "$runtime/node_modules/turbo/bin/turbo" "\$@"
EOF
chmod +x "$turbo"

# No turbo.lock is committed, so setup has to resolve one.
"$turbo" setup --no-frozen
"$turbo" --version

# Vercel CI hides the output of its planning step, which runs `turbo query`
# next. Run a query here so a failure shows up in the install log.
"$turbo" query '{ packages { items { name } } }' > /dev/null
