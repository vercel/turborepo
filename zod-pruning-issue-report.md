# Bun Lockfile Pruner - Zod Dependency Resolution Issue

## Executive Summary

The Bun lockfile pruner is incorrectly selecting `zod@4.1.12` as the default `"zod"` package entry when pruning for the `@bun-issue/web` workspace. This causes a critical version mismatch that breaks dependencies requiring `zod@3.x`. The correct behavior is to use `zod@3.25.76` as the default entry and create an aliased entry for packages that specifically need version 4.

## The Problem

### Current (Incorrect) Behavior - bad.lock

```json
"zod": ["zod@4.1.12", "", {}, "sha512-JInaHOamG8pt5+Ey8kGmdcAcg3OL9reK8ltczgHTAwNhMys/6ThXHityHxVV2p3fkw/c+MAvBHFVYHFZDmjMCQ=="]
```

### Expected (Correct) Behavior - good.lock

```json
"zod": ["zod@3.25.76", "", {}, "sha512-gzUt/qt81nXsFGKIFcC3YnfEAx5NkunCfnDlvuBSSFS02bcXu4Lmea0AFIUwbLWxWPx3d9p8S5QoaujKcNQxcQ=="],
...
"eslint-plugin-react-hooks/zod": ["zod@4.1.12", "", {}, "sha512-JInaHOamG8pt5+Ey8kGmdcAcg3OL9reK8ltczgHTAwNhMys/6ThXHityHxVV2p3fkw/c+MAvBHFVYHFZDmjMCQ=="]
```

## Root Cause Analysis

### Dependency Chain for @bun-issue/web

The `@bun-issue/web` workspace has the following relevant dependencies:

#### Dependencies Requiring zod@3.x

1. **@tanstack/router-plugin@1.134.12** (devDependency)
   - Declares: `"zod": "^3.24.2"`
   - **Hard requirement**: Must be version 3.x (≥3.24.2)
   - Located at: `bad.lock:268`

2. **@tanstack/router-generator@1.134.12** (transitive via router-plugin)
   - Declares: `"zod": "^3.24.2"`
   - **Hard requirement**: Must be version 3.x (≥3.24.2)
   - Located at: `bad.lock:266`

#### Dependencies with Flexible Requirements

3. **eslint-plugin-react-hooks@6.1.1** (devDependency)
   - Declares: `"zod": "^3.22.4 || ^4.0.0"`
   - **Flexible**: Can use either version 3.x OR 4.x
   - Located at: `bad.lock:392`

### Version Satisfaction Analysis

Given these requirements:

| Package | Requirement | Satisfied by 3.25.76? | Satisfied by 4.1.12? |
|---------|-------------|----------------------|---------------------|
| @tanstack/router-plugin | `^3.24.2` | ✅ Yes | ❌ **NO** |
| @tanstack/router-generator | `^3.24.2` | ✅ Yes | ❌ **NO** |
| eslint-plugin-react-hooks | `^3.22.4 \|\| ^4.0.0` | ✅ Yes | ✅ Yes |

**Conclusion**: `zod@3.25.76` satisfies ALL requirements, while `zod@4.1.12` breaks critical dependencies.

## Why the Pruner Chooses Incorrectly

The pruner is selecting `zod@4.1.12` as the default entry because:

1. **Presence in original lockfile**: The original `original-issue-11007-2.lock` contains `zod@4.1.12` at line 70 as the default entry, likely due to the `packages/validation` workspace having a peer dependency on `"zod": "^4.1.12"` (line 33).

2. **Algorithm priority issue**: When the pruner encounters multiple versions of the same package, it appears to prioritize based on:
   - **Incorrect heuristic**: Possibly selecting the version that appears first in the original lockfile, or the "highest" version number
   - **Missing constraint analysis**: Not properly analyzing which version satisfies ALL dependency constraints in the pruned dependency tree

3. **Lack of constraint intersection**: The algorithm should be computing the intersection of all version requirements for the pruned dependency tree and selecting the version that satisfies the most restrictive constraints (or all constraints if possible).

## Expected Behavior

The correct algorithm should:

1. **Collect all zod requirements** in the pruned dependency tree:
   - `^3.24.2` (from @tanstack/router-plugin)
   - `^3.24.2` (from @tanstack/router-generator)
   - `^3.22.4 || ^4.0.0` (from eslint-plugin-react-hooks)

2. **Identify available versions** in the original lockfile:
   - `zod@3.25.76` (if present in full lockfile)
   - `zod@4.1.12` (present at line 70)

3. **Determine default version** by finding the version that satisfies the most dependencies:
   - `3.25.76` satisfies: ✅ router-plugin, ✅ router-generator, ✅ eslint-plugin-react-hooks (via `^3.22.4`)
   - `4.1.12` satisfies: ❌ router-plugin, ❌ router-generator, ✅ eslint-plugin-react-hooks (via `^4.0.0`)
   - **Winner**: `3.25.76` (satisfies 3/3 vs 1/3)

4. **Create aliases for packages needing different versions**:
   - Since `eslint-plugin-react-hooks` can use either version but may have previously resolved to 4.x in the original lockfile, create an alias `"eslint-plugin-react-hooks/zod"` pointing to `zod@4.1.12`
   - This preserves the exact resolution from the original lockfile while ensuring the default entry works for strict requirements

## Impact

Using `zod@4.1.12` as the default causes:

1. **Build failures**: Any code importing from `@tanstack/router-plugin` or `@tanstack/router-generator` will fail at runtime if they transitively depend on zod APIs
2. **Version mismatch errors**: Type mismatches between expected zod v3 APIs and actual zod v4 APIs
3. **Incorrect lockfile state**: The pruned lockfile doesn't accurately represent a valid dependency resolution

## Root Cause in Code

**Primary Issue**: Package resolution performs **name-only lookups without semantic version validation**.

**Key Locations**:

1. **`crates/turborepo-lockfiles/src/bun/mod.rs:396-461`** (`resolve_package`)
   - Receives version specifier (e.g., `"^3.24.2"`) but never validates it
   - Calls `find_package` with only the package name

2. **`crates/turborepo-lockfiles/src/bun/index.rs:164`** (`find_package`)
   - Line 164: `if let Some(entry) = self.by_key.get(name)`
   - Returns first entry matching the name, regardless of version
   - No version constraint checking occurs

3. **`crates/turborepo-lockfiles/src/bun/mod.rs:350-391`** (`process_package_entry`)
   - Simply returns whatever entry was found
   - No validation that the version satisfies the original constraint

**The Algorithm Assumes**: "If a package name exists in the lockfile, it must satisfy the version requirement."

**Why This Fails**: When multiple versions exist (e.g., `zod@3.25.76` nested under `@tanstack/router-plugin/zod` and `zod@4.1.12` at top-level), it returns the top-level entry without checking version compatibility.

See [zod-pruning-root-cause-code-analysis.md](./zod-pruning-root-cause-code-analysis.md) for detailed code flow analysis.

## Recommended Fix

The Bun lockfile pruner needs to implement proper semantic version constraint solving:

1. **Add semantic version validation** to `resolve_package` method:
   - Parse the found package's actual version
   - Validate it against the version specifier using semver
   - If validation fails, search nested/aliased entries
   - Return the first version that satisfies the constraint

2. **Alternative: Use `by_ident` index** with constraint solving:
   - Get all available versions of a package
   - Filter to those satisfying the version spec
   - Select the most appropriate one

3. **For pruning**: Build a constraint satisfaction problem for each package with multiple versions:
   - Select the version that satisfies all constraints when possible
   - If no single version satisfies all, use the version that satisfies the most packages
   - Create aliases for packages needing different versions
   - Prioritize non-aliased usages for the default entry

## Test Case Verification

To verify the fix:

```bash
# Prune original-issue-11007-2.lock for @bun-issue/web
bun prune --workspace=@bun-issue/web

# Expected: default "zod" entry should be zod@3.25.76
# Expected: "eslint-plugin-react-hooks/zod" alias should be zod@4.1.12
```

## Additional Context

- Original lockfile: `crates/turborepo-lockfiles/src/bun/snapshots/original-issue-11007-2.lock`
- Expected output: `crates/turborepo-lockfiles/src/bun/snapshots/good.lock`
- Actual output: `crates/turborepo-lockfiles/src/bun/snapshots/bad.lock`
- Issue reference: #11007
