# 04 SIGKILL Parent Death

Tests: killing Turbo with `-9` should still clean up the spawned task tree.

Run from this directory:

```sh
../../../target/debug/turbo run dev --filter=app-a &
turbo_pid=$!
sleep 2
kill -9 "$turbo_pid"
sleep 2
```

Then check the spawned child is gone:

```sh
kill -0 "$(cat apps/app-a/child.pid)" 2>/dev/null && echo child-still-alive || echo child-gone
```

Expected: `child-gone`
