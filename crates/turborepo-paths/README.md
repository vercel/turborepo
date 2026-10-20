# turborepo-paths

## Purpose

Type-safe path handling for Turborepo. Defines distinct path types that encode important properties at the type level, preventing bugs from mixing incompatible path formats.

## Architecture

Four main path type families, each with borrowed (`Path`) and owned (`PathBuf`) variants:

| Type | Properties | Use Case |
|------|------------|----------|
| `AbsoluteSystemPath` | Absolute, platform separators | Filesystem operations |
| `AnchoredSystemPath` | Relative to repo root, platform separators | Paths within the repository |
| `RelativeUnixPath` | Relative, forward slashes | Cache keys, cross-platform storage |

```
AbsoluteSystemPath ──┐
                     ├── Filesystem I/O
AnchoredSystemPath ──┘
         │
         └── Can be joined to produce AbsoluteSystemPath

RelativeUnixPath ──── Cache storage (platform-independent)
```

Built on `camino` for UTF-8 path guarantees.

## Notes

All paths are validated as UTF-8. The type system prevents common bugs like:
- Using a relative path where absolute is required
- Storing platform-specific paths in cache artifacts
- Mixing Unix and Windows path separators incorrectly
