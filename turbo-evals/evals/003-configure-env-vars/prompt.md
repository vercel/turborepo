Configure environment variable handling in turbo.json for the build task.

1. Add "env" array to the build task with "NODE_ENV"
2. Add "globalEnv" at the root level with "CI"

This ensures proper cache invalidation when these environment variables change.
