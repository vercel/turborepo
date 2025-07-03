### Description

This PR fixes an issue where glob patterns with leading `./` prefixes (like `./packages/*`) were not being consistently handled in workspace filtering and glob matching throughout Turborepo.

**Problem:**

- Workspace globs like `./packages/*` and `packages/*` should be functionally equivalent
- The current implementation didn't normalize these patterns, causing inconsistencies in package filtering
- This led to scenarios where `turbo run build --filter="./packages/*"` might not match the same packages as `turbo run build --filter="packages/*"`

**Solution:**

- **Normalize leading `./` patterns in `fix_glob_pattern`**: Strip the `./` prefix from glob patterns during normalization while preserving `../` patterns (which have different semantic meaning)
- **Enhanced filtering compatibility**: Update the package filtering logic to handle both original and normalized patterns, ensuring compatibility with workspace globs that might have or lack the `./` prefix
- **Comprehensive test coverage**: Add test cases to validate that `./packages/*` and `packages/*` produce identical results

**Key Changes:**

1. **`crates/turborepo-globwalk/src/lib.rs`**: Modified `fix_glob_pattern()` to normalize leading `./` patterns
2. **`crates/turborepo-lib/src/run/scope/filter.rs`**: Enhanced filtering logic to try both original and normalized patterns
3. **`crates/turborepo-repository/src/inference.rs`**: Fixed test assertions for repo state inference

**Examples of patterns affected:**

- `./packages/*` → `packages/*`
- `./packages/**` → `packages/**`
- `../packages/*` → `../packages/*` (preserved - different semantic meaning)

This ensures consistent behavior across all workspace filtering operations and improves the developer experience by making glob patterns more predictable.

### Testing Instructions

1. **Basic functionality test**:

   ```bash
   # Both of these should now match identical packages
   turbo run build --filter="./packages/*"
   turbo run build --filter="packages/*"
   ```

2. **Run the test suite**:

   ```bash
   cargo test test_fix_glob_pattern
   cargo test test_leading_dot_slash_pattern_normalization
   ```

3. **Test workspace filtering**:

   ```bash
   # In a monorepo with packages in a `packages/` directory
   turbo run build --filter="./packages/*" --dry-run
   turbo run build --filter="packages/*" --dry-run
   # Output should be identical
   ```

4. **Verify edge cases**:
   ```bash
   # Should preserve ../ patterns (different semantic meaning)
   turbo run build --filter="../packages/*" --dry-run
   ```
