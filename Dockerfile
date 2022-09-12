# golang parameters
ARG GO_VERSION

FROM ghcr.io/goreleaser/goreleaser-cross-base:v${GO_VERSION} AS osx-cross-base
ENV OSX_CROSS_PATH=/osxcross
ARG DEBIAN_FRONTEND=noninteractive

# Install deps
SHELL ["/bin/bash", "-c"]
RUN \
    set -x; \
    echo "Starting image build for Debian" \
 && dpkgArch="$(dpkg --print-architecture)" \
 && dpkg --add-architecture amd64 \
 && dpkg --add-architecture arm64 \
 && dpkg --add-architecture armel \
 && dpkg --add-architecture armhf \
 && dpkg --add-architecture i386 \
 && dpkg --add-architecture mips \
 && dpkg --add-architecture mipsel \
 && dpkg --add-architecture powerpc \
 && dpkg --add-architecture ppc64el \
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
        crossbuild-essential-armel \
        crossbuild-essential-armhf \
        crossbuild-essential-mipsel \
        crossbuild-essential-ppc64el \
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

FROM osx-cross-base AS osx-cross
ARG OSX_CROSS_COMMIT
ARG OSX_SDK
ARG OSX_SDK_SUM
ARG OSX_VERSION_MIN

WORKDIR "${OSX_CROSS_PATH}"

COPY patches /patches

RUN \
    git clone https://github.com/tpoechtrager/osxcross.git . \
 && git config user.name "John Doe" \
 && git config user.email johndoe@example.com \
 && git checkout -q "${OSX_CROSS_COMMIT}" \
 && git am < /patches/libcxx.patch \
 && rm -rf ./.git

# install osxcross:
COPY tars/${OSX_SDK}.tar.xz "${OSX_CROSS_PATH}/tarballs/${OSX_SDK}.tar.xz"

RUN \
    echo "${OSX_SDK_SUM}" "${OSX_CROSS_PATH}/tarballs/${OSX_SDK}.tar.xz" | sha256sum -c - \
 && apt-get update \
 && apt-get install --no-install-recommends -y -q \
        autotools-dev \
        libxml2-dev \
        lzma-dev \
        libssl-dev \
        zlib1g-dev \
        libmpc-dev \
        libmpfr-dev \
        libgmp-dev \
        llvm-dev \
        uuid-dev \
        binutils-multiarch-dev \
 && apt -y autoremove \
 && apt-get clean \
 && rm -rf /var/lib/apt/lists/* /tmp/* /var/tmp/* \
 && UNATTENDED=1 OSX_VERSION_MIN=${OSX_VERSION_MIN} ./build.sh

FROM osx-cross-base AS final
LABEL maintainer="Artur Troian <troian dot ap at gmail dot com>"
LABEL "org.opencontainers.image.source"="https://github.com/goreleaser/goreleaser-cross"

ARG DEBIAN_FRONTEND=noninteractive

COPY --from=osx-cross "${OSX_CROSS_PATH}/target" "${OSX_CROSS_PATH}/target"
ENV PATH=${OSX_CROSS_PATH}/target/bin:$PATH
