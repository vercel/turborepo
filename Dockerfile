FROM dockercore/golang-cross

LABEL maintainer="Goren G<gythialy.koo+github@gmail.com>"

# install mips and arm gcc
RUN apt-get update -qq && \
      apt-get install -y -q --no-install-recommends \
	  gcc-mips-linux-gnu g++-mips-linux-gnu binutils-mips-linux-gnu \
	  libc6-dev-mips-cross libc6-dev-mipsel-cross linux-libc-dev-mips-cross \ 
	  gcc-arm-linux-gnueabi g++-arm-linux-gnueabi gcc-arm-linux-gnueabihf g++-arm-linux-gnueabihf \
	  libc6-dev-armel-cross libc6-dev-armel-cross binutils-arm-linux-gnueabi libncurses5-dev \
	  gcc-aarch64-linux-gnu g++-aarch64-linux-gnu && \
      apt-get clean && rm -rf /var/lib/apt/lists/* /tmp/* /var/tmp/* 

# install goreleaser
RUN  \
	GORELEASER_VERSION=0.118.2 && \
	GORELEASER_SHA=131937804698f57c0f22db833da7425d8a175df131f6cec4b06be1768058a2a1 && \
	GORELEASER_DOWNLOAD_FILE=goreleaser_Linux_x86_64.tar.gz && \
	GORELEASER_DOWNLOAD_URL=https://github.com/goreleaser/goreleaser/releases/download/v${GORELEASER_VERSION}/${GORELEASER_DOWNLOAD_FILE} && \
	wget ${GORELEASER_DOWNLOAD_URL}; \
			echo "$GORELEASER_SHA $GORELEASER_DOWNLOAD_FILE" | sha256sum -c - || exit 1; \
			tar -xzf $GORELEASER_DOWNLOAD_FILE -C /usr/bin/ goreleaser; \
			rm $GORELEASER_DOWNLOAD_FILE;

CMD ["goreleaser", "-v"]
