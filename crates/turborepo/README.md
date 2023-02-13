# turbo cli

## Build Requirement

1. Install `protobuf` and `golang`

- On macOS: `brew install protobuf protoc-gen-go protoc-gen-go-grpc golang`
- On Windows: `choco install protoc golang make python3 mingw`

2. `go install google.golang.org/protobuf/cmd/protoc-gen-go@v1.28.0`
3. `go install google.golang.org/grpc/cmd/protoc-gen-go-grpc@v1.2.0`

4. Setup Rust

- On macOS: `curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh`
- On Windows:
  - `choco install rustup.install`
  - Setup Rust to use MingW instead of MSVC:
    - For x86: `rustup set default-host x86_64-pc-windows-gnu`
    - For ARM: `aarch64-pc-windows-gnu`
