# turborepo-lockfile-hash

## Purpose

Cycle-free, byte-compatible hashing for normalized lockfile package closures.

## Architecture

The crate preserves the historical Cap'n Proto layout and xxHash64 output used
by Turborepo task hashes. Repository knowledge uses it to install package
resolution fingerprints during generation construction. `turborepo-hash`
delegates its compatibility wrappers to the same primitive.

Input order is preserved and affects the resulting 16-character lowercase
hexadecimal hash. Repository generation supplies lexicographic `(key, version)`
ordering; `turborepo-hash` compatibility wrappers intentionally preserve their
supplied order.
