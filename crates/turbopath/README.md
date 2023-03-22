# turbopath.rs

`turbopath` is a thin wrapper around `std::path` which surfaces additional path type information into the runtime.

## Path Types

The available types are the crossproduct of the types of paths you're likely to encounter in cross-platform application development. The library itself intentionally does no normalization, instead electing to defer that to the implementer.

### `Absolute` vs. `Anchored`

- Absolute: The path is fully-qualified.
- Anchored: The path is fully-specified from an (unspecified) anchor which _must_ be an Absolute path. It may contain arbitrary traversal behavior such that the resulting path is _not_ descended from its anchor.

### `CrossPlatform` vs. `System` vs. `Unix` vs. `Windows`

- `CrossPlatform`: only relevant for non-`Absolute` paths; specifies that each `Component`'s contents have safely-abstractable behavior in all contexts.
- `System`: dependent on the platform at runtime. Any `Path` that comes from the present system and `is_absolute` can be cast to this.
- `Unix`: `/`-rooted, `/`-separated paths.
- `Windows`: Prefixed (e.g. `C:\`), `\`-separated paths.

## API

### `turbopath::{AbsoluteSystemPath, AbsoluteSystemPathBuf}`

Usage is identical to `std::path::{Path, PathBuf}`, except:

- Method signatures have changed in order to enforce the type guarantees.
- More things are implemented as `TryFrom` instead of `AsRef`.

Upstream API features that are unavailable:

- `AbsoluteSystemPathBuf.from_iter()` cannot be safely implemented, and as such is excluded.

### Unimplemented Structs

`turbopath` will eventually include implementations for:

- `AnchoredSystemPath` & `AnchoredSystemPathBuf`: likely only an intermediary type
- `AnchoredUnixPath` & `AnchoredUnixPathBuf`: used for generating output that is cross-platform

As the need arises it is possible that additional structs will be implemented for:

- `AbsoluteUnixPath` & `AbsoluteUnixPathBuf`
- `AbsoluteWindowsPath` & `AbsoluteWindowsPathBuf`
- `AnchoredWindowsPath` & `AnchoredWindowsPathBuf`
- `Component::Normal` for `Unix`, `Windows`, `System`, and `CrossPlatform`

## License

This implementation of this is heavily influenced by both Rust's `std::path` and [`camino`](https://docs.rs/camino/latest/camino/). Given the thinness of the abstraction in most cases there are few actual choices available in the precise implementation.

Consequentially, the content of the first commit in this crate is available under the terms of either the [Apache 2.0 license](LICENSE-APACHE) or the [MIT license](LICENSE-MIT), matching that of both upstream sources. Copyrights belong to their respective owners.

All subsequent contributions are available only under a [MPL 2.0](LICENSE-MPL) license.
