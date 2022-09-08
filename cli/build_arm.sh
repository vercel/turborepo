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
  --platform linux/arm64 \
  --entrypoint /bin/bash \
  -it \
  $IMG

# docker run \
#   --rm \
#   --privileged \
#   -e CGO_ENABLED=1 \
#   -e GORELEASER_KEY=${GORELEASER_KEY} \
#   -v /var/run/docker.sock:/var/run/docker.sock \
#   -v $(dirname `pwd`):/go/src/turborepo \
#   -w /go/src/turborepo/cli \
#   --platform linux/arm64 \
#   $IMG \
#   build --rm-dist -f arm-release.yml --snapshot --id turbo-linux-arm64
# #  release  --rm-dist --snapshot -f arm-release.yml
