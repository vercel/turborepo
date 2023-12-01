# `@turbo/repository`

This package builds `@turbo/repository`, which in turn packages up some of Turborepo's repository analysis functionality
for use in a javascript context.

The `rust/` folder contains the wrapper around core Turborepo Rust code, and should limit
itself to basic data transformations to match JS APIs. Any logic updates should preferably land in core Turborepo.

The `js/` folder contains the meta package to handle importing platform-specific native libraries, as well as the type definitions
for the JS API.

This package contains scripts to build dev and release versions. `pnpm build && pnpm package` will build and package a dev version of the native library for `darwin-arm64`, or you can pass an additional argument for a specific target. `pnpm build:release` will build a release version of the library

# Publishing

There is now a version bump script in [bump-version.sh](./scripts/bump-version.sh). Passing it the new version will bump the meta package version, as well as the optional dependencies list and native packages.
