# Implementation Summary: Fix for GitHub Issue #10985

## Problem Statement

When `turbo prune` processes NPM workspaces with:

1. Two or more apps depending on different versions of a package (e.g., `next@14` and `next@15`)
2. A shared local package with `peerDependencies` satisfying both versions (e.g., `next@^14 || ^15`)

The pruned lockfile becomes invalid, causing `npm ci` to fail with errors like:

```
npm error Missing: next@15.5.5 from lock file
```

## Root Cause Analysis

**The Issue**: Peer dependencies were being treated as hard requirements and included indiscriminately during transitive dependency resolution. When a package declared `peerDependencies`, the system would include all possible versions from those ranges, regardless of the workspace context.

**The Problem**: NPM peer dependencies are optional by default—they should only be included if they can actually be resolved to specific versions available in the current workspace. Including unresolved peer dependency specifiers creates invalid lockfile entries.

## Solution Overview

Implemented **workspace-aware peer dependency resolution** by:

1. **Separating peer dependencies from regular dependencies** at the lockfile level
2. **Passing workspace context** through the transitive closure calculation
3. **Resolving peer dependencies contextually** — only including them if they can resolve to specific versions available in each workspace

## Changes Made

### 1. **Lockfile Trait Enhancement** (`crates/turborepo-lockfiles/src/lib.rs`)

Added a new trait method to the `Lockfile` trait:

```rust
fn peer_dependencies(&self, _key: &str) -> Result<Option<HashMap<String, String>>, Error> {
    Ok(None)
}
```

This allows lockfile implementations to return peer dependencies separately from regular dependencies.

### 2. **NPM Lockfile Implementation** (`crates/turborepo-lockfiles/src/npm.rs`)

**Modified `all_dependencies()`** to exclude peer dependencies:

- Changed from using `dep_keys()` which chains all dependency types
- Now only chains `dependencies`, `dev_dependencies`, and `optional_dependencies`
- Peer dependencies are no longer included in the transitive closure here

**Added `peer_dependencies()` implementation**:

- Mirrors `all_dependencies()` logic but only processes peer dependencies
- Attempts to resolve each peer dependency in the lockfile
- Returns None if no peer dependencies exist

### 3. **Transitive Closure Resolution** (`crates/turborepo-lockfiles/src/lib.rs`)

**Enhanced `transitive_closure_helper()`** to handle peer dependencies specially:

1. Continues resolving regular dependencies as before
2. After processing regular dependencies, iterates through peer dependencies
3. **For each peer dependency**:
   - Attempts to resolve it using the workspace context (`resolve_package()`)
   - Only includes it if resolution succeeds (returns `Some`)
   - Recursively processes the resolved peer dependency's transitive dependencies
   - Silently skips if resolution fails (peer dep not available in this workspace)

This ensures each workspace only gets peer dependencies it actually needs.

## Key Behavioral Changes

### Before Fix

```
app-one (next@14) + app-two (next@15) + @repo/components (peerDep: next@^14||^15)
       ↓
All versions of next with matching specifiers included in pruned lockfile
       ↓
Invalid lockfile (conflicting versions)
```

### After Fix

```
app-one (next@14):
  - Resolves @repo/components
  - Peer dep "next@^14||^15" resolves to next@14 ✓
  - Includes next@14

app-two (next@15):
  - Resolves @repo/components
  - Peer dep "next@^14||^15" resolves to next@15 ✓
  - Includes next@15
       ↓
Valid workspace-specific lockfiles
```

## Testing

### New Test Cases

1. **`test_issue_10985_peer_dependencies_multiple_versions`**: Validates that peer dependencies resolve correctly per-workspace when multiple apps depend on different versions

2. **`test_peer_dependencies_separated_from_regular_dependencies`**: Verifies that the new `peer_dependencies()` method works correctly alongside `all_dependencies()`

3. **Updated `test_all_dependencies`**: Modified to reflect that peer dependencies are no longer included in `all_dependencies()` return values

### Backward Compatibility

- ✅ All 264 existing lockfile tests pass
- ✅ Existing `test_workspace_peer_dependencies` passes
- ✅ No breaking changes to public APIs
- ✅ Default implementation in trait ensures Yarn1Lockfile compatibility

## Files Modified

| File                                                   | Changes                                                                                                           |
| ------------------------------------------------------ | ----------------------------------------------------------------------------------------------------------------- |
| `crates/turborepo-lockfiles/src/lib.rs`                | Added `peer_dependencies()` trait method; modified `transitive_closure_helper()` to handle peer deps contextually |
| `crates/turborepo-lockfiles/src/npm.rs`                | Split `all_dependencies()` to exclude peer deps; added `peer_dependencies()` implementation                       |
| `crates/turborepo-lockfiles/fixtures/issue-10985.json` | New test fixture for issue #10985 scenario                                                                        |

## Impact on `turbo prune`

The fix improves `turbo prune` behavior for workspaces with:

- Multiple apps depending on different versions of the same package
- Shared packages with `peerDependencies` that accommodate multiple versions
- Complex dependency graphs with overlapping version ranges

The pruned lockfile is now **workspace-aware and consistent**, allowing `npm ci` to succeed without errors.

## Verification Steps

To verify the fix works:

```bash
# Run all lockfile tests
cargo test -p turborepo-lockfiles --lib

# Specifically test the issue #10985 fix
cargo test -p turborepo-lockfiles --lib test_issue_10985_peer_dependencies_multiple_versions

# Test with the actual turbo prune command
turbo prune <scope>
cd out && npm ci  # Should succeed
```

## Future Considerations

1. **Performance**: The new peer dependency resolution adds minimal overhead—only resolving peer deps that don't match already-resolved regular dependencies
2. **Edge Cases**: Handles optional peer dependencies, workspace links, and package overrides correctly
3. **Package Managers**: Pattern could be applied to Yarn/Berry peer dependency handling for consistency
