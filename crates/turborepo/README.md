# turbo cli

## Build Requirement

1. Install `protobuf` and `golang` (note: Go must be pinned to v1.20.x, see https://github.com/vercel/turbo/issues/5918 for details)

- On macOS: `brew install protobuf protoc-gen-go protoc-gen-go-grpc go@1.20`
- On Windows: `choco install golang --version=1.20.7` and `choco install protoc make python3 mingw`
- On Ubuntu: `apt-get install golang golang-goprotobuf-dev`

2. `go install google.golang.org/protobuf/cmd/protoc-gen-go@v1.28.0`
3. `go install google.golang.org/grpc/cmd/protoc-gen-go-grpc@v1.2.0`
