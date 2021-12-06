# golang parameters
ARG GO_VERSION

FROM golang:${GO_VERSION}-bullseye AS base
LABEL maintainer="Artur Troian <troian dot ap at gmail dot com>"

ARG DEBIAN_FRONTEND=noninteractive
ARG GORELEASER_VERSION
ARG APT_MIRROR
ARG TINI_VERSION
ARG COSIGN_VERSION
ARG COSIGN_SHA256
ARG GORELEASER_DOWNLOAD_URL=https://github.com/goreleaser/goreleaser/releases/download/v${GORELEASER_VERSION}

# install cosign
COPY --from=gcr.io/projectsigstore/cosign:1.3.0 /bin/cosign /usr/local/bin/cosign
COPY entrypoint.sh /

# Install deps
RUN \
    set -x \
 && dpkgArch="$(dpkg --print-architecture)" \
 && curl --fail -sSL -o /tini https://github.com/krallin/tini/releases/download/${TINI_VERSION}/tini-$dpkgArch \
 && chmod +x /tini \
 && echo "Starting image build for Debian" \
 && sed -ri "s/(httpredir|deb).debian.org/${APT_MIRROR:-deb.debian.org}/g" /etc/apt/sources.list \
 && sed -ri "s/(security).debian.org/${APT_MIRROR:-security.debian.org}/g" /etc/apt/sources.list \
 && apt-get update \
 && apt-get install --no-install-recommends -y -q \
    software-properties-common \
 && curl -fsSL https://download.docker.com/linux/ubuntu/gpg | APT_KEY_DONT_WARN_ON_DANGEROUS_USAGE=1 apt-key add - \
 && echo "deb [arch=$dpkgArch] https://download.docker.com/linux/debian $(lsb_release -cs) stable" | tee /etc/apt/sources.list.d/docker.list \
 && apt-get update \
 && apt-get install --no-install-recommends -y -q \
        docker-ce \
        docker-ce-cli \
        make \
        git-core \
        wget \
        xz-utils \
        cmake \
        openssl \
 && apt -y autoremove \
 && apt-get clean \
 && rm -rf /var/lib/apt/lists/* /tmp/* /var/tmp/* \
 && GORELEASER_DOWNLOAD_FILE=goreleaser_${GORELEASER_VERSION}_${dpkgArch}.deb \
 && GORELEASER_DOWNLOAD_DEB="${GORELEASER_DOWNLOAD_URL}/${GORELEASER_DOWNLOAD_FILE}" \
 && cosign verify-blob --key ${GORELEASER_DOWNLOAD_URL}/cosign.pub \
    --signature "${GORELEASER_DOWNLOAD_URL}/checksums.txt.sig" "${GORELEASER_DOWNLOAD_URL}/checksums.txt" \
 && wget ${GORELEASER_DOWNLOAD_DEB} \
 && dpkg -i ${GORELEASER_DOWNLOAD_FILE} \
 && rm ${GORELEASER_DOWNLOAD_FILE} \
 && chmod +x /entrypoint.sh

ENTRYPOINT ["/tini", "--", "/entrypoint.sh"]

###############################################################################
FROM base AS osx-cross-base
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
ARG DEBIAN_FRONTEND=noninteractive

COPY --from=osx-cross "${OSX_CROSS_PATH}/target" "${OSX_CROSS_PATH}/target"
ENV PATH=${OSX_CROSS_PATH}/target/bin:$PATH
