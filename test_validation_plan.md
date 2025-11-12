# Test Validation Plan for Error Handling Fixes

## Quick Validation Commands

Run these commands to quickly validate the current state of error handling:

```bash
# 1. Run existing tests for the affected modules
cargo test -p turborepo-scm git::tests
cargo test -p turborepo-lib change_detector

# 2. Check for panics in error paths (should pass with the fixes)
RUST_BACKTRACE=1 cargo test -p turborepo-scm test_error_cases

# 3. Run with sanitizers to detect undefined behavior
RUSTFLAGS="-Z sanitizer=address" cargo test -p turborepo-scm --target x86_64-unknown-linux-gnu
```

## Manual Testing Scenarios

### Test Case 1: Non-UTF8 in Git Output

```bash
# Create a test repository with non-UTF8 filenames
mkdir test-repo && cd test-repo
git init

# Create a file with invalid UTF-8 in name (using Python)
python3 -c "import os; os.write(1, b'test\xff\xfe.txt')" > badfile

# Try to run turbo with change detection
turbo run build --filter=[HEAD^...HEAD]
# Should not panic, should fall back to all packages
```

### Test Case 2: Unanchorable Paths

```bash
# Create a git repo outside the turbo root
git init /tmp/external-repo
cd /tmp/external-repo
echo '{"name": "test"}' > package.json

# Try to use turbo with mismatched paths
TURBO_ROOT=/different/path turbo run build --filter=[HEAD^...HEAD]
# Should handle the path error gracefully
```

### Test Case 3: Corrupted Git Repository

```bash
# Create a normal repository
mkdir corrupt-test && cd corrupt-test
git init
echo "test" > file.txt
git add . && git commit -m "initial"

# Corrupt the git objects
find .git/objects -type f | head -1 | xargs -I {} sh -c 'echo "corrupted" > {}'

# Run turbo
turbo run build --filter=[HEAD^...HEAD]
# Should handle corruption gracefully
```

## Automated Test Suite Additions

### 1. Add to `crates/turborepo-scm/src/git.rs`

```rust
#[test]
fn test_invalid_utf8_handling() {
    // Test with actual invalid UTF-8 bytes
    let invalid_bytes = vec![0xFF, 0xFE, 0xFD];
    let result = String::from_utf8(invalid_bytes);
    assert!(result.is_err());

    // Verify our error type conversion works
    let error: Error = result.unwrap_err().into();
    assert!(matches!(error, Error::Encoding(_, _)));
}

#[test]
fn test_add_files_robustness() {
    // Test the actual function with invalid input
    // Should not panic, should return Error
}
```

### 2. Add to `crates/turborepo-lib/src/run/scope/change_detector.rs`

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_fallback_on_scm_error() {
        // Mock SCM to return various errors
        // Verify fallback behavior
    }

    #[test]
    fn test_warning_logs_emitted() {
        // Verify that warnings are logged when errors occur
    }
}
```

## Coverage Measurement

### Before Running Tests:

```bash
# Install coverage tools
cargo install cargo-tarpaulin

# Measure baseline coverage
cargo tarpaulin -p turborepo-scm -p turborepo-lib --out Html
```

### Target Coverage Goals:

- `add_files_from_stdout`: >90% coverage including error paths
- `changed_packages`: >85% coverage including all error branches
- `all_packages_changed_due_to_error`: 100% coverage

## Regression Testing

### Create Regression Test Suite:

```bash
#!/bin/bash
# regression_tests.sh

set -e

echo "Testing panic prevention for non-UTF8 paths..."
# Test 1: Non-UTF8 handling

echo "Testing path anchoring errors..."
# Test 2: Path errors

echo "Testing SCM error fallback..."
# Test 3: SCM errors

echo "All regression tests passed!"
```

## Performance Testing

Ensure error handling doesn't significantly impact performance:

```bash
# Benchmark normal operation
cargo bench --package turborepo-scm changed_files

# Benchmark with errors
cargo bench --package turborepo-scm changed_files_with_errors
```

## Continuous Integration

Add to CI pipeline:

```yaml
# .github/workflows/test-error-handling.yml
name: Error Handling Tests
on: [push, pull_request]
jobs:
  test:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v2

      - name: Run error handling tests
        run: |
          cargo test -p turborepo-scm test_invalid_utf8
          cargo test -p turborepo-scm test_path_errors
          cargo test -p turborepo-lib test_fallback

      - name: Run with invalid locale
        run: |
          LANG=C cargo test -p turborepo-scm

      - name: Fuzz testing
        run: |
          cargo fuzz run git_output_parser -- -max_total_time=60
```

## Checklist for PR Review

- [ ] All new error paths have tests
- [ ] No `.unwrap()` or `.expect()` on user input
- [ ] Error messages are helpful and actionable
- [ ] Logging at appropriate levels (warn for fallback)
- [ ] Documentation updated for error scenarios
- [ ] Performance impact measured
- [ ] Fuzz testing completed
- [ ] Integration tests pass
