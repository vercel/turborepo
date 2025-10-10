# Use the official Microsoft devcontainers Ubuntu base image
FROM mcr.microsoft.com/devcontainers/base:ubuntu-22.04

# Avoid warnings by switching to noninteractive
ENV DEBIAN_FRONTEND=noninteractive

# Update package list and install dependencies
RUN apt-get update && apt-get install -y \
    # Basic build tools
    build-essential \
    curl \
    wget \
    git \
    pkg-config \
    libssl-dev \
    # LLD linker (required for Linux as mentioned in CONTRIBUTING.md)
    lld \
    # Protocol Buffers compiler
    protobuf-compiler \
    # Cap'n Proto
    capnproto \
    libcapnp-dev \
    # Test dependencies (jq and zstd)
    jq \
    zstd \
    # Additional useful tools
    htop \
    vim \
    nano \
    unzip \
    && rm -rf /var/lib/apt/lists/*

# Install Node.js v22 using NodeSource repository (as root)
RUN curl -fsSL https://deb.nodesource.com/setup_22.x | bash - \
    && apt-get install -y nodejs

# Switch to vscode user for the rest of the setup
USER vscode

# Install pnpm with the specific version from package.json
RUN curl -fsSL https://get.pnpm.io/install.sh | env PNPM_VERSION=8.14.0 sh -

# Add pnpm to PATH
ENV PATH="/home/vscode/.local/share/pnpm:$PATH"

# Install Rust using rustup
RUN curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y --default-toolchain none

# Add Rust to PATH
ENV PATH="/home/vscode/.cargo/bin:$PATH"

# Install the specific Rust toolchain (nightly-2025-06-20) with required components
RUN /home/vscode/.cargo/bin/rustup toolchain install nightly-2025-06-20 --component rustfmt,clippy

# Set the default toolchain to match rust-toolchain.toml
RUN /home/vscode/.cargo/bin/rustup default nightly-2025-06-20

# Install llvm-tools-preview for coverage reporting
RUN /home/vscode/.cargo/bin/rustup component add llvm-tools-preview

# Install cargo-llvm-cov for coverage
RUN /home/vscode/.cargo/bin/cargo install cargo-llvm-cov

# Install other useful cargo tools
RUN /home/vscode/.cargo/bin/cargo install cargo-watch cargo-edit

# Create a custom cargo coverage command alias
RUN echo 'alias cargo-coverage="cargo llvm-cov --html --open"' >> /home/vscode/.bashrc

# Verify installations
RUN node --version && npm --version && pnpm --version
RUN /home/vscode/.cargo/bin/rustc --version && /home/vscode/.cargo/bin/cargo --version
RUN protoc --version && capnp --version
RUN jq --version && zstd --version

# Switch back to noninteractive to avoid warnings
ENV DEBIAN_FRONTEND=noninteractive