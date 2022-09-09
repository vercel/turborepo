#!/bin/sh

set -e

IMG=ghcr.io/gsoltis/turbo-cross:v1.18.5-arm64
#IMG=ghcr.io/gsoltis/goreleaser-cross:v1.18.5-arm64

docker run \
  --rm \
  --privileged \
  -e CGO_ENABLED=1 \
  -e GORELEASER_KEY=${GORELEASER_KEY} \
  -v /var/run/docker.sock:/var/run/docker.sock \
  -v $(dirname `pwd`):/go/src/turborepo \
  -w /go/src/turborepo/cli \
  --platform linux/amd64 \
  --entrypoint /bin/bash \
  -it \
  ghcr.io/goreleaser/goreleaser-cross:v1.18

# docker run \
#   --rm \
#   --privileged \
#   -e CGO_ENABLED=1 \
#   -e GORELEASER_KEY=${GORELEASER_KEY} \
#   -v /var/run/docker.sock:/var/run/docker.sock \
#   -v $(dirname `pwd`):/go/src/turborepo \
#   -w /go/src/turbo \
#   --platform linux/amd64 \
#   ghcr.io/goreleaser/goreleaser-cross:v1.18 \
#   release  --rm-dist --snapshot -f amd-release.yml
