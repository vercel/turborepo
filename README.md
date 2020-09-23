# golang-cross [![Actions Status](https://github.com/troian/golang-cross/workflows/Docker%20Image%20CI/badge.svg)](https://github.com/troian/golang-cross/actions)

Docker container to do cross compilation (Linux, Windows, macOS, ARM, ARM64) of go packages including support for cgo. 
Allows to cross-compile Golang with CGO include using of sysroot

Each goreleaser build entry may require an variable in order to properly compile and link with C/C++ libraries
- PKG_CONFIG_SYSROOT_DIR - path to sysroot. see examples in sysroot dir
- PKG_CONFIG_PATH - path to pkgconfig files

Check .goreleaser.yaml for complete example

## Docker
Find it on docker hub https://hub.docker.com/r/troian/golang-cross or run 

- generate env var from file
  ```bash
  export PRIVATE_KEY=$(cat ~/private_key.gpg | base64)
  ```

## How to run
See Makefile for examples 
