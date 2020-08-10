FROM dockercore/golang-cross

LABEL maintainer="Goren G<gythialy.koo+github@gmail.com>"

COPY entrypoint.sh /

# install arm gcc
RUN apt-get update -qq && apt-get install -y -q build-essential \
	  gcc-arm-linux-gnueabi g++-arm-linux-gnueabi gcc-arm-linux-gnueabihf g++-arm-linux-gnueabihf \
	  libc6-dev-armel-cross libc6-dev-armel-cross binutils-arm-linux-gnueabi libncurses5-dev \
	  gcc-aarch64-linux-gnu g++-aarch64-linux-gnu \
	  gcc-mingw-w64 g++-mingw-w64 && \
	  apt -y autoremove && \
    apt-get clean && rm -rf /var/lib/apt/lists/* /tmp/* /var/tmp/* 

# install goreleaser
RUN  \
	GORELEASER_VERSION=0.141.0 && \
	GORELEASER_SHA=00a00e7e7ea72c2bc44d1f1e50eccfac361e0a1332ee0fcde8107da442c3a4d8 && \
	GORELEASER_DOWNLOAD_FILE=goreleaser_Linux_x86_64.tar.gz && \
	GORELEASER_DOWNLOAD_URL=https://github.com/goreleaser/goreleaser/releases/download/v${GORELEASER_VERSION}/${GORELEASER_DOWNLOAD_FILE} && \
	wget ${GORELEASER_DOWNLOAD_URL}; \
			echo "$GORELEASER_SHA $GORELEASER_DOWNLOAD_FILE" | sha256sum -c - || exit 1; \
			tar -xzf $GORELEASER_DOWNLOAD_FILE -C /usr/bin/ goreleaser; \
			rm $GORELEASER_DOWNLOAD_FILE;

# update golang
RUN \
	GOLANG_VERSION=1.14.7 && \
	GOLANG_DIST=https://storage.googleapis.com/golang/go${GOLANG_VERSION}.linux-amd64.tar.gz \
	GOLANG_DIST_SHA=4a7fa60f323ee1416a4b1425aefc37ea359e9d64df19c326a58953a97ad41ea5 && \
	wget -O go.tgz "$GOLANG_DIST"; \
	echo "${GOLANG_DIST_SHA} *go.tgz" | sha256sum -c -; \
	rm -rf /usr/local/go; \
	tar -C /usr/local -xzf go.tgz; \
	rm go.tgz; 

RUN go get -u github.com/git-chglog/git-chglog/cmd/git-chglog && \
	chmod +x /entrypoint.sh

ENTRYPOINT ["bash", "/entrypoint.sh"]

# CMD ["goreleaser", "-v"]
