#!/bin/sh
set -eu

BUF_VERSION=v1.52.1
PROTOC_GEN_GO_VERSION=v1.36.11

SCRIPT_DIR="$(CDPATH='' cd -- "$(dirname -- "$0")" && pwd)"
ROOT="$(CDPATH='' cd -- "$SCRIPT_DIR/.." && pwd)"
BIN_DIR="$ROOT/.bin/${BUF_VERSION}-${PROTOC_GEN_GO_VERSION}"

install_tool() {
  binary="$1"
  package="$2"
  version="$3"
  if [ ! -x "$BIN_DIR/$binary" ]; then
    GOBIN="$BIN_DIR" go install "$package@$version"
  fi
}

mkdir -p "$BIN_DIR"
install_tool buf github.com/bufbuild/buf/cmd/buf "$BUF_VERSION"
install_tool protoc-gen-go google.golang.org/protobuf/cmd/protoc-gen-go "$PROTOC_GEN_GO_VERSION"

cd "$ROOT"
PATH="$BIN_DIR:$PATH" buf generate --template buf.gen.go.yaml
