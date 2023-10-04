# Turborepo FFI

This crate provides a C-compatible FFI for dependencies that are being
ported from Go to Rust. The dependencies use protobuf to send and receive
values from Go.

The crate produces a staticlib which is then linked to the Go code
in `cli/internal/ffi/ffi.go` using CGO.

## Common Questions

- Why do I get linker errors in Go when I use this crate?
  Can't I link C dependencies in Rust?

Because this crate produces a staticlib, it cannot link C dependencies.
This is because a staticlib is an _input_ to the linker and therefore
cannot bundle its C dependencies. Instead, you need to pass the libraries
to the CGO linker. You can do so by editing `cli/internal/ffi/ffi.go`,
where the linker flags are defined.

To find the libraries needed to link against, you can use rustc's `native-static-libs`
feature to print them.

For more information, read [here](https://users.rust-lang.org/t/solved-statically-linking-rust-library-yields-undefined-references/53815/5)
