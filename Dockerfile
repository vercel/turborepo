FROM dockercore/golang-cross

LABEL maintainer="Goren G<gythialy.koo+github@gmail.com>"

# install mips gcc
RUN apt-get update -qq && \
      apt-get install -y -q --no-install-recommends \
	  gcc-mips-linux-gnu g++-mips-linux-gnu binutils-mips-linux-gnu \
	  libc6-dev-mips-cross libc6-dev-mipsel-cross linux-libc-dev-mips-cross && \ 
      apt-get clean && rm -rf /var/lib/apt/lists/* /tmp/* /var/tmp/* 

# install goreleaser
RUN  \
	GORELEASER_VERSION=0.116.0 && \
	GORELEASER_SHA=34b7e3b843158bd0714d1be996951685496491adab4524fb1198ae144ab06ba3 && \
	GORELEASER_DOWNLOAD_FILE=goreleaser_Linux_x86_64.tar.gz && \
	GORELEASER_DOWNLOAD_URL=https://github.com/goreleaser/goreleaser/releases/download/v${GORELEASER_VERSION}/${GORELEASER_DOWNLOAD_FILE} && \
	wget ${GORELEASER_DOWNLOAD_URL}; \
			echo "$GORELEASER_SHA $GORELEASER_DOWNLOAD_FILE" | sha256sum -c - || exit 1; \
			tar -xzf $GORELEASER_DOWNLOAD_FILE -C /usr/bin/ goreleaser; \
			rm $GORELEASER_DOWNLOAD_FILE;

CMD ["goreleaser", "-v"]
