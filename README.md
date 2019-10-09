# golang-cross [![Actions Status](https://github.com/gythialy/golang-cross/workflows/Docker%20Image%20CI/badge.svg)](https://github.com/gythialy/golang-cross/actions)

Docker container to do cross compilation (Linux, windows, macOS, ARM, ARM64) of go packages including support for cgo. 

## Docker

Find it on docker hub https://hub.docker.com/r/goreng/golang-cross or run 

```
docker run --rm --privileged \
  -v $PWD:/go/src/github.com/qlcchain/go-qlc \
  -v /var/run/docker.sock:/var/run/docker.sock \
  -w /go/src/github.com/qlcchain/go-qlc \
  goreng/golang-cross goreleaser --snapshot --rm-dist
```
