# See here for image contents: https://github.com/microsoft/vscode-dev-containers/tree/v0.233.0/containers/go/.devcontainer/base.Dockerfile

# [Choice] Go version (use -bullseye variants on local arm64/Apple Silicon): 1, 1.16, 1.17, 1-bullseye, 1.16-bullseye, 1.17-bullseye, 1-buster, 1.16-buster, 1.17-buster
ARG VARIANT="1.18-bullseye"
FROM mcr.microsoft.com/vscode/devcontainers/go:0-${VARIANT}

RUN apt-get update && export DEBIAN_FRONTEND=noninteractive \
  && apt-get -y install --no-install-recommends \
  # Chromium for running Turbopack benchmarks
  chromium \
  # Used for plotters graph visualizations in turbopack benchmarks
  libfontconfig1-dev

# Add hyperfine, a useful benchmarking tool
RUN dpkgArch="$(dpkg --print-architecture)"; \
  wget "https://github.com/sharkdp/hyperfine/releases/download/v1.12.0/hyperfine_1.12.0_${dpkgArch}.deb" && dpkg -i "hyperfine_1.12.0_${dpkgArch}.deb"

#
# Everything below is run as the vscode user. If superuser permissions are necessary,
# run it before this. Otherwise, prefer running as the vscode user.
#
USER vscode

# [Choice] Node.js version: none, lts/*, 16, 14, 12, 10
ARG NODE_VERSION="none"
RUN if [ "${NODE_VERSION}" != "none" ]; then umask 0002 && sh -c ". /usr/local/share/nvm/nvm.sh && nvm install ${NODE_VERSION}" 2>&1; fi

RUN sh -c ". /usr/local/share/nvm/nvm.sh && npm install -g vercel yarn yalc pnpm nodemon" 2>&1

# The installer from https://rustup.rs/ homepage, with the following changes:
#   * `-y` to accept without interactivity
#   * `--default-toolchain none` to avoid installing stable rust. Our specific toolchain is installed post-create.
RUN curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y --default-toolchain none
