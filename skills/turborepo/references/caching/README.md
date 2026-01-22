# How Turborepo Caching Works

Turborepo's core principle: **never do the same work twice**.

## The Cache Equation

```
fingerprint(inputs) → stored outputs
```

If inputs haven't changed, restore outputs from cache instead of re-running the task.

## What Determines the Cache Key

### Global Hash Inputs

These affect ALL tasks in the repo:

- `package-lock.json` / `yarn.lock` / `pnpm-lock.yaml`
- Files listed in `globalDependencies`
- Environment variables in `globalEnv`
- `turbo.json` configuration

```json
{
  "globalDependencies": [".env", "tsconfig.base.json"],
  "globalEnv": ["CI", "NODE_ENV"]
}
```

### Task Hash Inputs

These affect specific tasks:

- All files in the package (unless filtered by `inputs`)
- `package.json` contents
- Environment variables in task's `env` key
- Task configuration (command, outputs, dependencies)
- Hashes of dependent tasks (`dependsOn`)

```json
{
  "tasks": {
    "build": {
      "dependsOn": ["^build"],
      "inputs": ["src/**", "package.json", "tsconfig.json"],
      "env": ["API_URL"]
    }
  }
}
```

## What Gets Cached

1. **File outputs** - files/directories specified in `outputs`
2. **Task logs** - stdout/stderr for replay on cache hit

```json
{
  "tasks": {
    "build": {
      "outputs": ["dist/**", ".next/**"]
    }
  }
}
```

## Local Cache Location

```
.turbo/cache/
├── <hash1>.tar.zst    # compressed outputs
├── <hash2>.tar.zst
└── ...
```

Add `.turbo` to `.gitignore`.

## Cache Restoration

On cache hit, Turborepo:

1. Extracts archived outputs to their original locations
2. Replays the logged stdout/stderr
3. Reports the task as cached (shows `FULL TURBO` in output)

## Example Flow

```bash
# First run - executes build, caches result
turbo build
# → packages/ui: cache miss, executing...
# → packages/web: cache miss, executing...

# Second run - same inputs, restores from cache
turbo build
# → packages/ui: cache hit, replaying output
# → packages/web: cache hit, replaying output
# → FULL TURBO
```

## Key Points

- Cache is content-addressed (based on input hash, not timestamps)
- Empty `outputs` array means task runs but nothing is cached
- Tasks without `outputs` key cache nothing (use `"outputs": []` to be explicit)
- Cache is invalidated when ANY input changes
