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

ENV SSL_VER="1.1.1q" \
    ZLIB_VER="1.2.13"

WORKDIR /musl-amd64
ENV PREFIX=/musl-amd64
RUN curl -sSL https://zlib.net/zlib-$ZLIB_VER.tar.gz | tar xz && \
    cd zlib-$ZLIB_VER && \
    CC="musl-gcc -fPIC -pie" LDFLAGS="-L$PREFIX/lib" CFLAGS="-I$PREFIX/include" ./configure --static --prefix=$PREFIX && \
    make -j$(nproc) && make install

RUN curl -sSL https://www.openssl.org/source/openssl-$SSL_VER.tar.gz | tar xz && \
    cd openssl-$SSL_VER && \
    ./Configure no-zlib no-shared -fPIC --prefix=$PREFIX --openssldir=$PREFIX/ssl linux-x86_64 && \
    env C_INCLUDE_PATH=$PREFIX/include make depend 2> /dev/null && \
    make -j$(nproc) && make install

WORKDIR /cross
RUN wget https://more.musl.cc/11.2.1/x86_64-linux-musl/aarch64-linux-musl-cross.tgz
RUN tar -xvf aarch64-linux-musl-cross.tgz
ENV CC="/cross/aarch64-linux-musl-cross/bin/aarch64-linux-musl-gcc -fPIC -pie"
WORKDIR /musl-arm64

ENV PREFIX=/musl-arm64
RUN curl -sSL https://zlib.net/zlib-$ZLIB_VER.tar.gz | tar xz && \
    cd zlib-$ZLIB_VER && \
    LDFLAGS="-L$PREFIX/lib" CFLAGS="-I$PREFIX/include" ./configure --static --prefix=$PREFIX && \
    make -j$(nproc) && make install

RUN curl -sSL https://www.openssl.org/source/openssl-$SSL_VER.tar.gz | tar xz && \
    cd openssl-$SSL_VER && \
    ./Configure no-zlib no-shared -fPIC --prefix=$PREFIX --openssldir=$PREFIX/ssl linux-aarch64 && \
    env C_INCLUDE_PATH=$PREFIX/include make depend 2> /dev/null && \
    make -j$(nproc) && make install

ENV CC=
