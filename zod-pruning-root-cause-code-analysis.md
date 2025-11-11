# Bun Lockfile Pruner - Root Cause Code Analysis: Zod Version Selection

## Executive Summary

The Bun lockfile pruner incorrectly selects `zod@4.1.12` instead of `zod@3.25.76` when pruning for `@bun-issue/web` because the package resolution algorithm performs **name-only lookups without semantic version validation**. When multiple versions of a package exist in the lockfile, it returns whichever entry matches the name first, regardless of whether it satisfies the version constraint.

## Code Flow Analysis

### 1. Entry Point: Transitive Closure Computation

**Location**: `crates/turborepo-lockfiles/src/lib.rs:178-220`

```rust
fn transitive_closure_helper_impl<L: Lockfile + ?Sized>(
    lockfile: &L,
    workspace_path: &str,
    unresolved_deps: HashMap<String, impl AsRef<str>>,
    resolved_deps: &mut HashSet<Package>,
    ignore_missing_packages: bool,
) -> Result<(), Error> {
    for (name, specifier) in unresolved_deps {
        let pkg = match lockfile.resolve_package(workspace_path, &name, specifier.as_ref()) {
            Ok(pkg) => pkg,
            // ... error handling
        };
        // ... continue walking dependencies
    }
    Ok(())
}
```

**What happens:**
- Called with `name: "zod"`, `specifier: "^3.24.2"` (from @tanstack/router-plugin)
- Calls `lockfile.resolve_package(workspace_path, "zod", "^3.24.2")`
- **Critical Issue**: Passes the version specifier, but resolution doesn't validate it

### 2. Package Resolution Entry Point

**Location**: `crates/turborepo-lockfiles/src/bun/mod.rs:396-461`

```rust
fn resolve_package(
    &self,
    workspace_path: &str,
    name: &str,
    version: &str,  // ← "^3.24.2" passed in but NOT validated!
) -> Result<Option<crate::Package>, crate::Error> {
    let workspace_entry = self.data.workspaces.get(workspace_path)
        .ok_or_else(|| crate::Error::MissingWorkspace(workspace_path.into()))?;
    let workspace_name = &workspace_entry.name;

    // ... catalog resolution, overrides handling (lines 409-442)

    // Try workspace-scoped lookup first
    if let Some(entry) = self.index.get_workspace_scoped(workspace_name, name)
        && let Some(pkg) = self.process_package_entry(entry, name, override_version, resolved_version)?
    {
        return Ok(Some(pkg));  // ← Returns without version validation
    }

    // Try finding via the general find_package method (includes bundled)
    if let Some((_key, entry)) = self.index.find_package(Some(workspace_name), name)
        && let Some(pkg) = self.process_package_entry(entry, name, override_version, resolved_version)?
    {
        return Ok(Some(pkg));  // ← Returns without version validation
    }

    Ok(None)
}
```

**Critical Issue**:
- The `version` parameter (`"^3.24.2"`) is passed to `resolve_package`
- BUT it's never used to validate which package version to return
- The code only uses `name` to look up packages
- It returns the first matching package entry by name, regardless of version compatibility

### 3. Package Lookup Strategy

**Location**: `crates/turborepo-lockfiles/src/bun/index.rs:148-178`

```rust
pub fn find_package<'a>(
    &'a self,
    workspace: Option<&str>,
    name: &'a str,  // ← Only searches by NAME, not NAME+VERSION
) -> Option<(&'a str, &'a PackageEntry)> {
    // Try workspace-scoped first
    if let Some(ws) = workspace {
        let lookup_key = (Arc::from(ws), Arc::from(name));
        if let Some(key) = self.workspace_scoped.get(&lookup_key)
            && let Some(entry) = self.by_key.get(key)
        {
            return Some((key.as_ref(), entry));
        }
    }

    // Try top-level
    if let Some(entry) = self.by_key.get(name) {
        return Some((name, entry));  // ← RETURNS FIRST MATCH BY NAME ONLY
    }

    // Try bundled (search all parents)
    for ((_parent, dep_name), key) in &self.bundled_deps {
        if dep_name.as_ref() == name
            && let Some(entry) = self.by_key.get(key)
        {
            return Some((key.as_ref(), entry));
        }
    }

    None
}
```

**Critical Issue**:
- Line 164: `if let Some(entry) = self.by_key.get(name)`
- This looks up by lockfile key name only (e.g., `"zod"`)
- Returns whatever "zod" entry exists in the lockfile
- **No version checking occurs**

### 4. Entry Processing (Where Version Validation SHOULD Happen)

**Location**: `crates/turborepo-lockfiles/src/bun/mod.rs:350-391`

```rust
fn process_package_entry(
    &self,
    entry: &PackageEntry,
    name: &str,
    override_version: &str,
    resolved_version: &str,
) -> Result<Option<crate::Package>, crate::Error> {
    let ident = PackageIdent::parse(&entry.ident);

    // Filter out workspace mapping entries
    if ident.is_workspace() {
        return Ok(None);
    }

    // Check for overrides
    if override_version != resolved_version {
        let override_ident = format!("{name}@{override_version}");
        if let Some((_override_key, override_entry)) = self.index.get_by_ident(&override_ident)
        {
            // ... return override entry
        }
    }

    // Return the package with its version (and patch if applicable)
    let mut version = entry.version().to_string();
    if let Some(patch) = self.data.patched_dependencies.get(&entry.ident) {
        version.push('+');
        version.push_str(patch);
    }
    Ok(Some(crate::Package {
        key: entry.ident.to_string(),  // ← Just returns whatever was found
        version,
    }))
}
```

**Critical Issue**:
- This function receives the found `entry` and just returns it
- **No validation** that `entry.version()` satisfies the version specifier
- Lines 381-390: Just extracts the version and returns it
- The version specifier is completely ignored at this point

### 5. Index Structure Shows Alternative Lookup Method Exists

**Location**: `crates/turborepo-lockfiles/src/bun/index.rs:102-111`

```rust
/// Get a package entry by ident (e.g., "lodash@4.17.21").
///
/// If multiple keys map to the same ident, returns the first one
/// (which is typically the workspace-scoped one due to sorting).
pub fn get_by_ident(&self, ident: &str) -> Option<(&str, &PackageEntry)> {
    let keys = self.by_ident.get(ident)?;
    let key = keys.first()?;
    let entry = self.by_key.get(key)?;
    Some((key.as_ref(), entry))
}
```

**Note**: This method DOES support looking up by full ident like `"zod@3.25.76"`, but it's **not used** during package resolution. It's only used for override lookups (line 367 in mod.rs).

## Concrete Example: How Zod Resolution Fails

### Scenario: Resolving zod dependency for @tanstack/router-plugin

**Original Lockfile State** (`original-issue-11007-1.lock`):
```json
{
  "packages": {
    "zod": ["zod@4.1.12", "", {}, "sha512-..."],  // ← Top-level default
    "@tanstack/router-plugin/zod": ["zod@3.25.76", "", {}, "sha512-..."]  // ← Nested for tanstack
  }
}
```

**Dependency Chain**:
1. `@bun-issue/web` → devDep → `@tanstack/router-plugin@1.134.12`
2. `@tanstack/router-plugin@1.134.12` → dep → `"zod": "^3.24.2"`

**Resolution Flow**:

```
Step 1: transitive_closure_helper_impl
├─ name: "zod"
├─ specifier: "^3.24.2"  // ← Requires version 3.x
└─ Calls: resolve_package("apps/web", "zod", "^3.24.2")

Step 2: resolve_package (mod.rs:396)
├─ workspace_name: "@bun-issue/web"
├─ Tries: index.get_workspace_scoped("@bun-issue/web", "zod")
│  └─ Returns: None (no "apps/web/zod" entry exists)
├─ Tries: index.find_package(Some("@bun-issue/web"), "zod")
│  └─ Calls find_package...

Step 3: find_package (index.rs:148)
├─ Tries workspace-scoped: workspace_scoped.get(("@bun-issue/web", "zod"))
│  └─ Returns: None
├─ Tries top-level: by_key.get("zod")
│  └─ Returns: Some(PackageEntry { ident: "zod@4.1.12", ... })
│      ^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^
│      PROBLEM: Returns zod@4.1.12 without checking if it satisfies ^3.24.2
└─ Returns: ("zod", entry)

Step 4: process_package_entry (mod.rs:350)
├─ entry.ident: "zod@4.1.12"
├─ NO VERSION VALIDATION AGAINST "^3.24.2"
└─ Returns: Package { key: "zod@4.1.12", version: "4.1.12" }

Result: zod@4.1.12 added to transitive closure
        ^^^^^^^^^^
        WRONG VERSION! Should be 3.25.76
```

## Why The Algorithm Fails

### Assumption Violation

**Algorithm Assumes**:
> "If a package name exists in the lockfile at the requested scope, it must satisfy the version requirement."

**Why This Is Wrong**:
- When multiple versions of a package exist (e.g., zod@3.25.76 and zod@4.1.12)
- The top-level entry may be for a different workspace's requirements
- No validation ensures the found version satisfies the requester's version constraint

### Missing Validation Step

The algorithm should:
1. Find candidate package entries by name
2. **Parse the entry's actual version** (e.g., "4.1.12")
3. **Validate against the version specifier** (e.g., "^3.24.2")
4. If validation fails, **continue searching** for other versions (e.g., nested entries)
5. Return the first matching version, or error if none satisfy

**Current behavior**: Steps 2-4 are completely missing.

## Impact on Pruning

### Subgraph Construction

**Location**: `crates/turborepo-lockfiles/src/bun/mod.rs:946-1048`

The `subgraph` method receives a list of package identities from the transitive closure:

```rust
fn subgraph(
    &self,
    workspace_packages: &[String],
    packages: &[String],  // ← Includes "zod@4.1.12" (wrong version!)
) -> Result<BunLockfile, Error> {
    // ...
    for pkg in packages {
        // Lines 986-1024: Find matching lockfile keys for this package ident
        if self.data.packages.contains_key(pkg) {
            keys_to_include.insert(pkg.clone());
        } else if let Some(at_pos) = pkg.rfind('@') {
            // Search for entries matching this ident
            for (lockfile_key, entry) in &self.data.packages {
                if &entry.ident != pkg {
                    continue;
                }
                // Include this key
                keys_to_include.insert(lockfile_key.clone());
            }
        }
    }
    // ...
}
```

**Result**:
- The closure includes `"zod@4.1.12"` (wrong version)
- Subgraph finds the lockfile key `"zod"` that has ident `"zod@4.1.12"`
- Includes it in the pruned lockfile
- The correct nested entry `"@tanstack/router-plugin/zod": ["zod@3.25.76", ...]` is never considered

## Correct Behavior (as seen in good.lock)

The correct pruned lockfile should:

1. **Analyze all zod requirements** in the pruned dependency tree:
   - `@tanstack/router-plugin`: requires `^3.24.2` → MUST be 3.x
   - `@tanstack/router-generator`: requires `^3.24.2` → MUST be 3.x
   - `eslint-plugin-react-hooks`: requires `^3.22.4 || ^4.0.0` → CAN be either

2. **Select the version that satisfies ALL packages**:
   - `zod@3.25.76` satisfies: ✅ 3/3 packages
   - `zod@4.1.12` satisfies: ✅ 1/3 packages (only eslint-plugin-react-hooks)
   - **Winner**: `zod@3.25.76`

3. **Promote the winning version to top-level**:
   ```json
   "zod": ["zod@3.25.76", "", {}, "sha512-..."]
   ```

4. **Create aliases for packages that used the other version**:
   ```json
   "eslint-plugin-react-hooks/zod": ["zod@4.1.12", "", {}, "sha512-..."]
   ```

## Proposed Fixes

### Option 1: Add Semantic Version Validation to resolve_package

**Location**: `crates/turborepo-lockfiles/src/bun/mod.rs:396-461`

```rust
fn resolve_package(
    &self,
    workspace_path: &str,
    name: &str,
    version_spec: &str,
) -> Result<Option<crate::Package>, crate::Error> {
    // ... existing catalog/override logic ...

    // Try workspace-scoped lookup
    if let Some(entry) = self.index.get_workspace_scoped(workspace_name, name)
        && let Some(pkg) = self.process_package_entry(entry, name, override_version, resolved_version)?
        && self.satisfies_version_spec(&pkg.version, version_spec)?  // ← ADD THIS
    {
        return Ok(Some(pkg));
    }

    // Try hoisted lookup
    if let Some((_key, entry)) = self.index.find_package(Some(workspace_name), name)
        && let Some(pkg) = self.process_package_entry(entry, name, override_version, resolved_version)?
        && self.satisfies_version_spec(&pkg.version, version_spec)?  // ← ADD THIS
    {
        return Ok(Some(pkg));
    }

    // NEW: Try nested/aliased versions if hoisted didn't satisfy
    if let Some(pkg) = self.find_nested_version_match(workspace_name, name, version_spec)? {
        return Ok(Some(pkg));
    }

    Ok(None)
}
```

### Option 2: Use by_ident Index with Constraint Solving

**Location**: `crates/turborepo-lockfiles/src/bun/index.rs:102-111`

Enhance `get_by_ident` to support partial matching:

```rust
/// Get all package entries that match a name, regardless of version
pub fn get_all_versions(&self, name: &str) -> Vec<(&str, &PackageEntry)> {
    let mut results = Vec::new();
    for (ident_key, keys) in &self.by_ident {
        if ident_key.starts_with(&format!("{name}@")) {
            for key in keys {
                if let Some(entry) = self.by_key.get(key) {
                    results.push((key.as_ref(), entry));
                }
            }
        }
    }
    results
}
```

Then in `resolve_package`:

```rust
// Get all versions of this package
let candidates = self.index.get_all_versions(name);

// Find the first one that satisfies the version spec
for (_key, entry) in candidates {
    let pkg_version = entry.version();
    if semver::Version::parse(pkg_version).ok()
        .and_then(|v| semver::VersionReq::parse(version_spec).ok()
            .map(|req| req.matches(&v)))
        .unwrap_or(false)
    {
        return Ok(Some(self.process_package_entry(entry, name, ...)?));
    }
}
```

### Option 3: Post-Processing Optimization in Subgraph

After the initial transitive closure, but before final pruning:

**Location**: After `crates/turborepo-lockfiles/src/lib.rs:207`

Add a validation pass:

```rust
// Validate that all package versions satisfy their constraints
for pkg in &resolved_deps {
    // Get the dependency that required this package
    // Check if the version satisfies the original constraint
    // If not, search for an alternative version in the lockfile
    // Replace in resolved_deps
}
```

## Recommended Fix

**Option 1** (add semantic version validation) is the most surgical and correct fix:

1. **Minimal code changes**: Only affects `resolve_package` and helpers
2. **Addresses root cause**: Validates versions at resolution time
3. **Preserves existing behavior**: For packages with only one version
4. **Handles multi-version packages**: Searches nested entries when top-level doesn't satisfy

## Test Case to Verify Fix

```rust
#[test]
fn test_prune_selects_correct_zod_version() {
    // Given: Lockfile with zod@3.25.76 and zod@4.1.12
    //        Workspace requires "zod": "^3.24.2"

    let lockfile = BunLockfile::load("original-issue-11007-1.lock")?;
    let pruned = lockfile.subgraph(&["apps/web"], &calculated_deps)?;

    // Then: Pruned lockfile should have zod@3.25.76 as default
    assert_eq!(
        pruned.packages.get("zod").unwrap().ident,
        "zod@3.25.76"
    );

    // And: zod@4.1.12 may exist as a nested entry if needed
    // but should NOT be the default top-level entry
}
```

## Summary

**Root Cause**: Package resolution performs name-only lookups without semantic version validation.

**Location**: `crates/turborepo-lockfiles/src/bun/mod.rs:396-461` (resolve_package) and `crates/turborepo-lockfiles/src/bun/index.rs:148-178` (find_package)

**Impact**: When multiple versions exist, the pruner selects whichever matches by name first, ignoring version constraints, leading to broken dependency resolutions in pruned lockfiles.

**Fix**: Add semantic version validation during package resolution to ensure the returned package version satisfies the requested version specifier.
