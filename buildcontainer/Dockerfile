ARG BASE_IMAGE
FROM ${BASE_IMAGE}
ARG DEBIAN_FRONTEND=noninteractive

# Install deps
SHELL ["/bin/bash", "-c"]
RUN \
    set -x; \
    echo "Starting image build for Debian" \
 && dpkgArch="$(dpkg --print-architecture)" \
 && dpkg --add-architecture amd64 \
 && dpkg --add-architecture arm64 \
 && apt-get update \
 && apt-get install --no-install-recommends -y -q \
        autoconf \
        automake \
        bc \
        python \
        binfmt-support \
        binutils-multiarch \
        build-essential \
        clang \
        gcc \
        g++ \
        gdb \
        mingw-w64 \
        crossbuild-essential-amd64 \
        crossbuild-essential-arm64 \
        devscripts \
        libtool \
        llvm \
        multistrap \
        patch \
        mercurial \
        musl-tools \
 && apt -y autoremove \
 && apt-get clean \
 && rm -rf /var/lib/apt/lists/* \
    /tmp/* \
    /var/tmp/* \
    rm -rf /usr/share/man/* \
    /usr/share/doc

# install a copy of mingw with aarch64 support to enable windows on arm64
WORKDIR /llvm-mingw
ARG TARGETARCH
RUN \
    if [ ${TARGETARCH} = "arm64" ]; then MINGW_ARCH=aarch64; else MINGW_ARCH=x86_64; fi && \
    wget "https://github.com/mstorsjo/llvm-mingw/releases/download/20220906/llvm-mingw-20220906-ucrt-ubuntu-18.04-${MINGW_ARCH}.tar.xz" && \
    tar -xvf llvm-mingw-20220906-ucrt-ubuntu-18.04-${MINGW_ARCH}.tar.xz && \
    ln -s llvm-mingw-20220906-ucrt-ubuntu-18.04-${MINGW_ARCH} llvm-mingw

ARG MAINTAINER
ARG GIT_REPO
LABEL maintainer=${MAINTAINER}
LABEL "org.opencontainers.image.source"=${GIT_REPO}
