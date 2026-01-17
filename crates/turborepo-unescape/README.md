# turborepo-unescape

## Purpose

Provides `UnescapedString`, a marker type for biome-parsed JSON strings that handles escape sequence processing. This exists because biome's JSON parser doesn't process escape sequences (see [biome#1596](https://github.com/biomejs/biome/issues/1596)).

## Architecture

```
JSON with escapes (e.g., "hello\nworld")
        │
        ▼
┌─────────────────────────────┐
│   biome_deserialize         │  ← Parses JSON, but leaves escapes as-is
└─────────────────────────────┘
        │
        ▼
┌─────────────────────────────┐
│   UnescapedString           │  ← Wraps string in quotes, uses serde_json
│   (Deserializable impl)     │     to process escapes like \n, \t, \uXXXX
└─────────────────────────────┘
        │
        ▼
    Properly unescaped String
```

The type implements `Deref<Target=String>`, so it can be used anywhere a `String` is expected. It's also transparent for serde, JSON Schema, and TypeScript generation.

## Notes

- Use `UnescapedString` instead of `String` in biome-deserialized config structs where escape sequences may appear
- The unescape logic wraps the raw string in quotes and re-parses with `serde_json` to leverage its escape handling
- Marked `#[serde(transparent)]` so it serializes/deserializes as a plain string
