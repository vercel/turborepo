# 02 Force Twice

Tests: first `Ctrl+C` starts graceful shutdown, second `Ctrl+C` force-kills the
task tree.

Run from this directory:

```sh
../../../target/debug/turbo run dev --filter=app-a
```

Then press `Ctrl+C`, wait about 3 seconds, press `Ctrl+C` again.

Expected:

```text
^C - Shutting down Turborepo tasks...
Some tasks in your Turborepo are taking awhile to shut down: app-a#dev
Press CTRL+C again to force shut down, or wait.
^C - Force killing Turborepo tasks: app-a#dev
```

Check the spawned child is gone:

```sh
kill -0 "$(cat apps/app-a/child.pid)" 2>/dev/null && echo child-still-alive || echo child-gone
```
