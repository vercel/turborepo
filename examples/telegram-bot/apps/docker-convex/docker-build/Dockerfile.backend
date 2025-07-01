# syntax=docker/dockerfile:1-labs
# This Dockerfile builds the self-hosted version of the Convex backend
# It creates a minimal runtime image that can run a local Convex instance

ARG VERGEN_GIT_SHA
ARG VERGEN_GIT_COMMIT_TIMESTAMP

# cargo-chef is used to cache Rust dependencies and build artifacts
# It creates a recipe of our dependencies first, then builds them in a separate stage
# This helps avoid rebuilding dependencies when only first-party source code changes
FROM lukemathwalker/cargo-chef:latest-rust-1 AS chef
WORKDIR /tmp
COPY rust-toolchain .
# Dummy command to just fetch the toolchain that'll get installed by running `cargo chef`
RUN rustup self update && rustup toolchain install
COPY --parents crates* Cargo* ./
RUN cargo chef prepare --recipe-path recipe.json --bin convex-local-backend

# Build stage: Compiles the Rust backend with all dependencies
# Uses cargo-chef's recipe from the previous stage for optimal caching
FROM lukemathwalker/cargo-chef:latest-rust-1 AS build
SHELL ["/bin/bash", "-o", "pipefail", "-c"]

# Update APT configuration for Docker builds
# Sets retry attempts, assumes yes for prompts, and configures dpkg timeout
# Also installs a few libraries needed for building the backend
RUN <<EOF
printf 'APT::Acquire::Retries \"10\";\n' > /etc/apt/apt.conf.d/80retries
printf 'APT::Get::Assume-Yes \"true\";\n' > /etc/apt/apt.conf.d/90forceyes
printf 'DPkg::Lock::Timeout \"30\";\n' > /etc/apt/apt.conf.d/85timeout
echo 'DEBIAN_FRONTEND=noninteractive' > /etc/environment
apt-get update
apt-get install -y --no-install-recommends cmake libclang-dev libstdc++6 libc6
EOF

# Install Just (for build scripts)
RUN cargo install just

WORKDIR /

# Install Node.js from NodeSource repository
# We extract the version from .nvmrc
COPY .nvmrc /nvmrc
RUN <<EOF
# Extract major version (e.g., "18" from "18.x.x")
NODE_MAJOR=$(cat /nvmrc | cut -d. -f1 | tr -d 'v\n')
# Add NodeSource repository and its signing key
curl -fsSL https://deb.nodesource.com/setup_${NODE_MAJOR}.x | bash -
# Install the specific version listed in the nvmrc file
NODE_VERSION=$(cat /nvmrc | tr -d 'v\n')
apt-get install -y --no-install-recommends nodejs=${NODE_VERSION}-1nodesource1
EOF

# Ensure this file is copied so we'll restart here if the file changes
COPY rust-toolchain /rust-toolchain
# Dummy command to just fetch the toolchain and cargo index
RUN rustup self update && rustup toolchain install && cargo search -q convex

WORKDIR /convex
COPY scripts ./scripts
RUN npm ci --prefix scripts/

# Disable build cache as it's not usable without a real git repo present
ENV RUSH_BUILD_CACHE_ENABLED=0
COPY npm-packages ./npm-packages
COPY Justfile ./
RUN --mount=type=cache,target=/convex/npm-packages/common/temp/,sharing=locked just rush install

COPY --from=chef /tmp/recipe.json recipe.json
# Use cargo-chef to build dependencies from the recipe created in the first stage
# This step will be cached unless dependencies change
RUN --mount=type=cache,target=/convex/target/ --mount=type=cache,target=/usr/local/cargo/git/db --mount=type=cache,target=/usr/local/cargo/registry/ cargo chef cook --release --recipe-path recipe.json
# Now copy everything else over -- we've minimized the set of files that cause a full `rush install` or `cargo build` to run.
COPY . .

# Build the convex-local-backend binary
# Must redeclare these as environment variables so that they are available
ARG VERGEN_GIT_SHA
ARG VERGEN_GIT_COMMIT_TIMESTAMP
ENV VERGEN_GIT_SHA=${VERGEN_GIT_SHA}
ENV VERGEN_GIT_COMMIT_TIMESTAMP=${VERGEN_GIT_COMMIT_TIMESTAMP}
RUN --mount=type=cache,target=/convex/npm-packages/common/temp/,sharing=locked --mount=type=cache,target=/convex/target/ --mount=type=cache,target=/usr/local/cargo/git/db --mount=type=cache,target=/usr/local/cargo/registry/ <<EOF
cargo build --release -p local_backend --bin convex-local-backend
cp target/release/convex-local-backend .
cargo build --release -p keybroker --bin generate_key
cp target/release/generate_key .
EOF
ARG debug
RUN if [[ -z "$debug" ]]; then strip ./convex-local-backend; strip ./generate_key; fi

# Final stage: Creates minimal runtime image with only necessary components
# Uses Ubuntu Noble (24.04) as the base image
FROM ubuntu:noble
ARG VERGEN_GIT_SHA
LABEL org.opencontainers.repository=https://github.com/get-convex/convex-backend
LABEL org.opencontainers.image.revision=${VERGEN_GIT_SHA}

WORKDIR /convex

# Install libraries needed for running the backend
RUN --mount=type=cache,target=/var/cache/apt --mount=type=cache,target=/var/lib/apt,sharing=locked <<EOF
apt-get update
apt-get install -y --no-install-recommends libclang1 curl ca-certificates
EOF

# Install Node.js and npm, required for running Node.js actions in the backend
COPY --from=build /usr/bin/node /usr/bin/node
COPY --from=build /usr/bin/npm /usr/bin/npm
COPY --from=build /usr/lib/node_modules/npm /usr/lib/node_modules/npm
# Set up npm and npx commands
RUN ln -sf /usr/lib/node_modules/npm/bin/npm-cli.js /usr/bin/npm && \
    chmod +x /usr/bin/npm
RUN ln -sf /usr/lib/node_modules/npm/bin/npx-cli.js /usr/bin/npx && \
    chmod +x /usr/bin/npx
COPY --from=build --chmod=744 /convex/convex-local-backend .
COPY --from=build --chmod=744 /convex/generate_key .
COPY --chmod=744 self-hosted/docker-build/read_credentials.sh .
COPY --chmod=744 self-hosted/docker-build/run_backend.sh .
COPY --chmod=744 self-hosted/docker-build/generate_admin_key.sh .

VOLUME /convex/data

# Set the backend as the executable
ENTRYPOINT ["./run_backend.sh"]

# Expose the required ports
EXPOSE 3210
EXPOSE 3211