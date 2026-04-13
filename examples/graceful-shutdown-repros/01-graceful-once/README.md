# 01 Graceful Once

Tests: first `Ctrl+C` should allow task cleanup to run.

Run from this directory:

```sh
../../../target/debug/turbo run dev --filter=app-a
```

Then press `Ctrl+C` once.

Expected:

```text
^C - Shutting down Turborepo tasks...
app-a:dev: graceful cleanup start
app-a:dev: graceful cleanup done
```

Check cleanup marker:

```sh
test -f apps/app-a/cleanup.done && echo cleanup-ran
```
