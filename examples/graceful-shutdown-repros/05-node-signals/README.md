# 05 Node Signals

Tests shutdown behavior with a real Node.js app plus a spawned Node.js worker.

Important:

- `SIGINT` and `SIGTERM` are handled by the app.
- `SIGKILL` cannot be trapped by Node.js or any other Unix process.
- This fixture writes `apps/app-a/sigkill-unhandleable.txt` at startup to make
  that explicit.

Run from this directory:

```sh
../../../target/debug/turbo run dev --filter=app-a
```

What the app does:

- parent process: `server.js`
- child worker: `worker.js`
- writes markers in `apps/app-a/`

Useful modes:

1. Graceful once

```sh
../../../target/debug/turbo run dev --filter=app-a
```

Press `Ctrl+C` once.

Expected:

- `node parent received SIGINT`
- `node worker received SIGINT`
- files like `sigint.parent` and `sigint.worker`

2. Force twice

```sh
NODE_STUBBORN=1 WORKER_STUBBORN=1 ../../../target/debug/turbo run dev --filter=app-a --env-mode=loose
```

Press `Ctrl+C`, wait about 3 seconds, then press `Ctrl+C` again.

Expected:

- `^C - Shutting down Turborepo tasks...`
- `Some tasks in your Turborepo are taking awhile to shut down: app-a#dev`
- `Press CTRL+C again to force shut down, or wait.`
- then `^C - Force killing Turborepo tasks: app-a#dev`

3. Parent death via `kill -9`

```sh
NODE_STUBBORN=1 WORKER_STUBBORN=1 ../../../target/debug/turbo run dev --filter=app-a &
turbo_pid=$!
sleep 2
kill -9 "$turbo_pid"
sleep 2
kill -0 "$(cat apps/app-a/worker.pid)" 2>/dev/null && echo worker-still-alive || echo worker-gone
```

Expected:

- `worker-gone`
