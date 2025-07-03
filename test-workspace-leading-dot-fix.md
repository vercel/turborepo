# Fix for Workspace Package Scoping with Leading `./` Patterns

## Issue Summary

**GitHub Issue**: [#8599 - Package scoping fails when workspace glob has leading `./`](https://github.com/vercel/turborepo/issues/8599)

### Problem Description

When workspace globs in `package.json` have leading `./` patterns (e.g., `"./packages/foo"`), Turborepo's package scoping/filtering fails to match packages correctly. Users cannot filter packages using patterns like `packages/bar` when the workspace glob is defined as `./packages/bar`.

### Root Cause

The issue stems from inconsistent normalization between:
1. **Workspace Discovery** (TypeScript): Uses workspace globs as-is with leading `./`
2. **Package Filtering** (Rust): Expects normalized paths without leading `./`

When a user runs `turbo run test --filter=packages/bar`, the filter tries to match against workspace paths that were discovered with leading `./` patterns, causing a mismatch.

## Solution Implemented

### 1. Updated `fix_glob_pattern` Function

Modified `crates/turborepo-globwalk/src/lib.rs` to normalize leading `./` patterns during glob processing:

```rust
pub fn fix_glob_pattern(pattern: &str) -> String {
    // Normalize leading ./ patterns for consistent workspace matching
    let normalized_pattern = if pattern.starts_with("./") {
        &pattern[2..]
    } else {
        pattern
    };
    
    let converted = Path::new(normalized_pattern)
        .to_slash()
        .expect("failed to roundtrip through Path");
    // ... rest of the function
}
```

### 2. Enhanced Filter Matching Logic

Updated `crates/turborepo-lib/src/run/scope/filter.rs` to handle both normalized and non-normalized patterns during filtering:

```rust
// Create normalized version for compatibility
let normalized_parent_dir_globber = parent_dir_unix
    .as_deref()
    .and_then(|path| {
        if path.starts_with("./") {
            let normalized = &path[2..];
            wax::Glob::new(normalized).ok()
        } else if !path.starts_with('.') {
            // Also try with ./ prefix to match workspace globs that might have it
            let with_prefix = format!("./{}", path);
            wax::Glob::new(&with_prefix).ok()
        } else {
            None
        }
    });

// Use both patterns for matching
let matches_original = parent_dir_globber.is_match(path.as_path());
let matches_normalized = normalized_parent_dir_globber
    .as_ref()
    .map(|g| g.is_match(path.as_path()))
    .unwrap_or(false);

if matches_original || matches_normalized {
    // Package matches either pattern
}
```

### 3. Added Comprehensive Test Coverage

Added test cases to verify the fix works correctly:

```rust
#[test_case("./packages/*", "packages/*" ; "normalize leading dot slash")]
#[test_case("./packages/**", "packages/**" ; "normalize leading dot slash with doublestar")]
#[test_case("../packages/*", "../packages/*" ; "preserve leading dotdot slash")]
#[test_case(
    vec![TargetSelector {
        parent_dir: Some(AnchoredSystemPathBuf::try_from("./packages/*").unwrap()),
        ..Default::default()
    }],
    None,
    &["project-0", "project-1"] ;
    "select by parentDir using glob with leading dot slash"
)]
```

## Testing Verification

### Test Case 1: Glob Pattern Normalization
```bash
cargo test -p globwalk test_fix_glob_pattern
```
✅ **PASSED**: All glob normalization tests pass, including new tests for leading `./` patterns.

### Test Case 2: Filter Matching
The fix ensures that both of these scenarios work:

**Scenario A**: Workspace defined as `./packages/*`, filter as `packages/bar`
**Scenario B**: Workspace defined as `packages/*`, filter as `./packages/bar`

## Benefits

1. **Backward Compatibility**: Existing workspaces continue to work unchanged
2. **Forward Compatibility**: New workspaces with leading `./` work correctly
3. **Flexible Filtering**: Users can use either pattern format in filters
4. **Consistent Behavior**: Matches npm's behavior for workspace resolution

## Implementation Details

### Files Modified

1. **`crates/turborepo-globwalk/src/lib.rs`**
   - Updated `fix_glob_pattern()` to normalize leading `./` patterns
   - Added test cases for glob normalization

2. **`crates/turborepo-lib/src/run/scope/filter.rs`**
   - Enhanced `filter_nodes_with_selector()` for dual pattern matching
   - Enhanced `filter_subtrees_with_selector()` for dual pattern matching
   - Added test case for leading dot slash filtering

### Key Functions

- `fix_glob_pattern()`: Normalizes workspace globs during parsing
- `filter_nodes_with_selector()`: Matches filters against package paths
- `filter_subtrees_with_selector()`: Matches filters in dependency trees

## Edge Cases Handled

1. **Mixed Patterns**: Some workspace globs with `./`, others without
2. **Complex Globs**: Patterns like `./packages/**` and `./packages/*`
3. **Relative Paths**: Preserves `../` patterns while normalizing `./`
4. **Filter Variations**: Users can filter with or without leading `./`

## Conclusion

This fix resolves the issue described in #8599 by ensuring consistent workspace glob normalization and flexible filter matching. The solution maintains backward compatibility while enabling the expected behavior for workspaces with leading `./` patterns.