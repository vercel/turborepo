# Local development image for Turborepo.
#
# The pinned versions below match the factory image the agents run on
# (apps/factory/agent/lib/factory-image.ts) and the CI setup actions
# (.github/actions/setup-*). `apps/factory` has a test that fails when
# this file drifts from that specification, so update them together.
FROM mcr.microsoft.com/vscode/devcontainers/base:ubuntu-22.04

# Set environment variables
ENV DEBIAN_FRONTEND=noninteractive
ENV RUSTUP_HOME=/usr/local/rustup
ENV CARGO_HOME=/usr/local/cargo
ENV PATH=/usr/local/cargo/bin:$PATH
ENV RUST_VERSION=nightly-2026-05-22
ENV NODE_MAJOR=24
ENV PNPM_VERSION=10.28.0
ENV PROTOC_VERSION=26.1
ENV ZIG_VERSION=0.15.2
ENV ZIG_GLOBAL_CACHE_DIR=/usr/local/zig-cache

# Install system dependencies
RUN apt-get update && apt-get install -y \
    # Build essentials
    build-essential \
    pkg-config \
    # LLD linker (.cargo/config.toml links with -fuse-ld=lld on Linux)
    lld \
    # Cap'n Proto
    capnproto \
    libcapnp-dev \
    # Testing utilities (optional but useful)
    jq \
    zstd \
    # SSL development libraries
    libssl-dev \
    # Additional utilities
    curl \
    wget \
    git \
    unzip \
    xz-utils \
    ca-certificates \
    gnupg \
    lsb-release \
    && rm -rf /var/lib/apt/lists/* \
    && (command -v ld.lld > /dev/null \
    || ln -sf "$(command -v lld)" /usr/local/bin/ld.lld)

# Install Node.js (using NodeSource repository for exact version control)
RUN curl -fsSL "https://deb.nodesource.com/setup_${NODE_MAJOR}.x" | bash - \
    && apt-get install -y nodejs \
    && rm -rf /var/lib/apt/lists/*

# Install pnpm globally (the version in the root package.json)
RUN npm install -g "pnpm@${PNPM_VERSION}"

# Install protoc, pinned to the version CI installs
RUN arch="$(uname -m)" \
    && case "$arch" in \
    x86_64 | amd64) asset="x86_64" ;; \
    aarch64 | arm64) asset="aarch_64" ;; \
    *) echo "unsupported architecture: $arch" >&2; exit 1 ;; \
    esac \
    && curl --fail --show-error --silent --location --output /tmp/protoc.zip \
    "https://github.com/protocolbuffers/protobuf/releases/download/v${PROTOC_VERSION}/protoc-${PROTOC_VERSION}-linux-${asset}.zip" \
    && unzip -o -q /tmp/protoc.zip -d /usr/local 'bin/protoc' 'include/*' \
    && chmod 0755 /usr/local/bin/protoc \
    && rm /tmp/protoc.zip

# Install Zig, required to build libghostty-vt for the terminal UI
RUN arch="$(uname -m)" \
    && case "$arch" in \
    x86_64 | amd64) zig_arch="x86_64" ;; \
    aarch64 | arm64) zig_arch="aarch64" ;; \
    *) echo "unsupported architecture: $arch" >&2; exit 1 ;; \
    esac \
    && curl --fail --show-error --silent --location --output /tmp/zig.tar.xz \
    "https://ziglang.org/download/${ZIG_VERSION}/zig-${zig_arch}-linux-${ZIG_VERSION}.tar.xz" \
    && mkdir -p /usr/local/zig \
    && tar -xJf /tmp/zig.tar.xz -C /usr/local/zig --strip-components 1 \
    && ln -sf /usr/local/zig/zig /usr/local/bin/zig \
    && mkdir -p "$ZIG_GLOBAL_CACHE_DIR" \
    && chmod -R a+rwX "$ZIG_GLOBAL_CACHE_DIR" \
    && rm /tmp/zig.tar.xz

# Install Rust with the repository toolchain and components
RUN curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- \
    --default-toolchain ${RUST_VERSION} \
    --profile minimal \
    --component rustfmt,clippy \
    -y \
    && chmod -R a+w $RUSTUP_HOME $CARGO_HOME

# Install the performance tooling the agent skills reach for
RUN cargo install --locked hyperfine cargo-bloat twiggy \
    && chmod -R a+w $CARGO_HOME

# Create cache directories for better performance
RUN mkdir -p /home/vscode/.cache /usr/local/cargo \
    && chown -R vscode:vscode /home/vscode/.cache /usr/local/cargo

# Set the default user to vscode
USER vscode

# Verify installations
RUN node --version \
    && npm --version \
    && pnpm --version \
    && rustc --version \
    && cargo --version \
    && protoc --version \
    && capnp --version \
    && zig version \
    && jq --version \
    && zstd --version \
    && hyperfine --version \
    && which ld.lld

WORKDIR /workspace
