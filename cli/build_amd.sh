#!/bin/sh

set -e

# docker run \
#   --rm \
#   --privileged \
#   -e CGO_ENABLED=1 \
#   -e GORELEASER_KEY=${GORELEASER_KEY} \
#   -v /var/run/docker.sock:/var/run/docker.sock \
#   -v `pwd`:/go/src/turbo \
#   -w /go/src/turbo \
#   --platform linux/amd64 \
#   --entrypoint /bin/bash \
#   -it \
#   ghcr.io/goreleaser/goreleaser-cross:v1.18

docker run \
  --rm \
  --privileged \
  -e CGO_ENABLED=1 \
  -e GORELEASER_KEY=${GORELEASER_KEY} \
  -v /var/run/docker.sock:/var/run/docker.sock \
  -v `pwd`:/go/src/turbo \
  -w /go/src/turbo \
  --platform linux/amd64 \
  ghcr.io/goreleaser/goreleaser-cross:v1.18 \
  release  --rm-dist --snapshot -f amd-release.yml
