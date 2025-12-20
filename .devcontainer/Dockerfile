# Use Ubuntu 22.04 LTS as base image
FROM mcr.microsoft.com/vscode/devcontainers/base:ubuntu-22.04

# Set environment variables
ENV DEBIAN_FRONTEND=noninteractive
ENV RUSTUP_HOME=/usr/local/rustup
ENV CARGO_HOME=/usr/local/cargo
ENV PATH=/usr/local/cargo/bin:$PATH
ENV RUST_VERSION=nightly-2025-06-20

# Install system dependencies
RUN apt-get update && apt-get install -y \
    # Build essentials
    build-essential \
    pkg-config \
    # LLD linker (required on Linux per CONTRIBUTING.md)
    lld \
    # Protocol Buffers compiler
    protobuf-compiler \
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
    ca-certificates \
    gnupg \
    lsb-release \
    && rm -rf /var/lib/apt/lists/*

# Install Node.js v22 (using NodeSource repository for exact version control)
RUN curl -fsSL https://deb.nodesource.com/setup_22.x | bash - \
    && apt-get install -y nodejs \
    && rm -rf /var/lib/apt/lists/*

# Install pnpm globally (specific version as per package.json)
RUN npm install -g pnpm@8.14.0

# Install Rust with the specific nightly version and components
RUN curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- \
    --default-toolchain ${RUST_VERSION} \
    --profile minimal \
    --component rustfmt,clippy \
    -y \
    && chmod -R a+w $RUSTUP_HOME $CARGO_HOME

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
    && jq --version \
    && zstd --version \
    && which lld

WORKDIR /workspace