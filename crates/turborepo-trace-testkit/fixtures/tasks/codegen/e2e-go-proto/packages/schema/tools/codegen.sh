#!/bin/sh
# Go bindings for the schema, built with tools pinned here.
set -e

pkg=$(cd "$(dirname "$0")/.." && pwd)
tools="$pkg/.tools"
mkdir -p "$tools"

for tool in github.com/bufbuild/buf/cmd/buf@v1.50.0 google.golang.org/protobuf/cmd/protoc-gen-go@v1.36.6; do
  GOBIN="$tools" go install "$tool"
done

cd "$pkg"
PATH="$tools:$PATH" buf generate --template buf.gen.go.yaml
