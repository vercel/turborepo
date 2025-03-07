# turborepo-lib

This crate contains most of the logic for the Turborepo binary and should only be consumed by the `turbo` crate.

During the Go to Rust migration, we put most of the Turborepo logic in this crate, and left `turbo` as a thin wrapper
that built the Go code. That way, we could build all of our Rust code without triggering a Go build as well.
Since the migration is done, there's no real reason to keep this split, but we haven't removed it yet.
