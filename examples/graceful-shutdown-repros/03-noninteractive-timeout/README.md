# 03 Noninteractive Timeout

Tests: no second `Ctrl+C` is possible, so Turbo should auto-force-kill after
about 10 seconds.

Run from this directory:

```sh
( exec </dev/null; exec ../../../target/debug/turbo run dev --filter=app-a ) &
turbo_pid=$!
sleep 2
kill -SIGINT "$turbo_pid"
time wait "$turbo_pid"
```

Expected:

```text
Shutting down Turborepo tasks...
Some tasks in your Turborepo are taking awhile to shut down: app-a#dev
Shutting down forcibly in 7s...
Graceful shutdown timed out. Force killing Turborepo tasks: app-a#dev
```

Check the spawned child is gone:

```sh
kill -0 "$(cat apps/app-a/child.pid)" 2>/dev/null && echo child-still-alive || echo child-gone
```
