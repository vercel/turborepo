# getTurboConfigs Refactor: Behavior Analysis

## Issue Summary

The refactor of `getTurboConfigs.ts` to eliminate duplicate logic for handling both `turbo.json` and `turbo.jsonc` configuration files has introduced a **subtle but significant behavior change** that could be causing macOS test failures.

## Key Behavior Change Identified

### Original Implementation (before refactor)
In the `getWorkspaceConfigs` function:

```typescript
// Try and get turbo.json or turbo.jsonc
const turboJsonPath = path.join(workspacePath, "turbo.json");
const turboJsoncPath = path.join(workspacePath, "turbo.jsonc");

// Check if both files exist
const turboJsonExists = fs.existsSync(turboJsonPath);
const turboJsoncExists = fs.existsSync(turboJsoncPath);

if (turboJsonExists && turboJsoncExists) {
  const errorMessage = `Found both turbo.json and turbo.jsonc in the same directory: ${workspacePath}\nPlease use either turbo.json or turbo.jsonc, but not both.`;
  logger.error(errorMessage);
  throw new Error(errorMessage);
}

let rawTurboJson = null;
let turboConfig: SchemaV1 | undefined;

try {
  if (turboJsonExists) {
    rawTurboJson = fs.readFileSync(turboJsonPath, "utf8");
  } else if (turboJsoncExists) {
    rawTurboJson = fs.readFileSync(turboJsoncPath, "utf8");
  }
  // ... rest of processing
}
```

### Refactored Implementation (current)
In the `getWorkspaceConfigs` function:

```typescript
// Try and get turbo.json or turbo.jsonc
const { configPath: turboConfigPath, configExists } = resolveTurboConfigPath(workspacePath);

let rawTurboJson = null;
let turboConfig: SchemaV1 | undefined;

try {
  if (configExists && turboConfigPath) {
    rawTurboJson = fs.readFileSync(turboConfigPath, "utf8");
  }
  // ... rest of processing
}
```

## Critical Difference: Error Handling Location

### Original Behavior
- The conflict check (both files exist) happened **within the main processing loop**
- If both files existed, an error was thrown **immediately** 
- The error would be caught by the outer `catch (e)` block that logs a warning and continues processing other workspaces
- **Result**: The function would continue processing other workspaces and return partial results

### Refactored Behavior  
- The conflict check happens **inside the `resolveTurboConfigPath` utility function**
- The utility function calls `logger.error()` and throws an error **immediately**
- This error is **NOT caught** by any try-catch block in the calling code
- **Result**: The entire `getWorkspaceConfigs` function fails and throws, stopping all processing

## Why This Affects macOS Specifically

### Potential macOS-Specific Issues

1. **Case Sensitivity**: macOS has case-insensitive but case-preserving filesystem by default
   - A test might create files with different cases that appear as duplicates
   - Example: `turbo.json` and `TURBO.JSON` would be seen as the same file on macOS but different on Linux

2. **File System Race Conditions**: macOS filesystem operations may have different timing characteristics
   - Tests that create/delete files quickly might encounter different race conditions
   - The `both-configs` fixture has both `turbo.json` and `turbo.jsonc` files

3. **Path Resolution Differences**: Different path resolution behavior between platforms
   - Symlinks, case handling, or path normalization differences

## Impact Assessment

### Before Refactor (Resilient)
```typescript
// If both configs exist in workspace A, log warning and continue
// Process workspace B, C, D normally  
// Return partial results with workspaces B, C, D
```

### After Refactor (Fragile)
```typescript
// If both configs exist in workspace A, throw error immediately
// Stop processing entirely
// Return nothing / throw to caller
```

## Evidence Supporting This Analysis

1. **Test Fixture Exists**: `packages/turbo-utils/__fixtures__/common/both-configs/` contains both `turbo.json` and `turbo.jsonc`

2. **Error Message Changed**: 
   - Original: `"Please use either turbo.json or turbo.jsonc, but not both."`
   - Refactored: `"Please use either turbo.json or turbo.jsonc, but not both."`
   - Same message, but different error handling behavior

3. **No Try-Catch Around New Function**: The `resolveTurboConfigPath` call is not wrapped in try-catch, so errors propagate up

## Recommended Fix

### Option 1: Restore Original Error Handling (Safest)
Move the conflict detection back into the calling code with proper error handling:

```typescript
// In getWorkspaceConfigs
try {
  const { configPath: turboConfigPath, configExists } = resolveTurboConfigPath(workspacePath);
  // ... rest of processing
} catch (e) {
  // Log warning and continue processing other workspaces (original behavior)
  logger.warn(e);
  // Still push the workspace config without turboConfig
  configs.push({
    workspaceName,
    workspacePath,
    isWorkspaceRoot,
    turboConfig: undefined, // No config due to conflict
  });
  continue;
}
```

### Option 2: Make Utility Function Non-Throwing
Modify `resolveTurboConfigPath` to return error information instead of throwing:

```typescript
function resolveTurboConfigPath(dirPath: string): {
  configPath: string | null;
  configExists: boolean;
  error?: string;
} {
  // ... check logic
  if (turboJsonExists && turboJsoncExists) {
    const errorMessage = `Found both turbo.json and turbo.jsonc in the same directory: ${dirPath}\nPlease use either turbo.json or turbo.jsonc, but not both.`;
    logger.error(errorMessage);
    return { configPath: null, configExists: false, error: errorMessage };
  }
  // ... rest
}
```

## Conclusion

The refactor successfully consolidated duplicate logic but inadvertently changed error handling behavior from **graceful degradation** to **fail-fast**. This change is likely causing tests to fail on macOS where the `both-configs` scenario or similar filesystem conflicts occur more frequently due to platform-specific filesystem behavior.

The fix should restore the original resilient error handling while maintaining the consolidated logic structure.