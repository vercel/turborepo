# 06 Multi App Signals

Three apps, one `turbo run dev`, and configurable signal behavior per app.

Run from this directory with the locally built binary:

```sh
../../../target/debug/turbo run dev --filter=app-a --filter=app-b --filter=app-c
```

Each app reads its own mode env var:

- `app-a` -> `APP_A_MODE`
- `app-b` -> `APP_B_MODE`
- `app-c` -> `APP_C_MODE`

Supported modes:

- `graceful`: logs the signal and exits after ~0.5s
- `slow`: logs the signal and exits after ~5s
- `stubborn`: logs the signal and stays alive until Turbo force-kills it
- `default`: installs no signal handlers, so Node uses its default behavior

## Scenario 1: all apps handle `SIGINT`

```sh
APP_A_MODE=graceful APP_B_MODE=graceful APP_C_MODE=graceful \
  ../../../target/debug/turbo run dev --filter=app-a --filter=app-b --filter=app-c
```

Press `Ctrl+C` once.

Expected:

- all three apps log `received SIGINT`
- all three apps exit cleanly
- no slow-shutdown warning appears

## Scenario 2: one app handles `SIGINT`, one app does not, one app is stubborn

```sh
APP_A_MODE=default APP_B_MODE=graceful APP_C_MODE=stubborn \
  ../../../target/debug/turbo run dev --filter=app-a --filter=app-b --filter=app-c
```

Press `Ctrl+C`, wait about 3 seconds, then press `Ctrl+C` again.

Expected:

- `app-a` exits on the first signal with no app-level signal log
- `app-b` logs `received SIGINT` and exits cleanly
- `app-c` logs `received SIGINT` and stays alive
- the slow-shutdown warning names only `app-c#dev`

## Scenario 3: one app is just slow

```sh
APP_A_MODE=slow APP_B_MODE=graceful APP_C_MODE=graceful \
  ../../../target/debug/turbo run dev --filter=app-a --filter=app-b --filter=app-c
```

Press `Ctrl+C` once.

Expected:

- `app-a` logs that it is taking awhile to exit
- `app-b` and `app-c` exit promptly
- after 3 seconds Turbo warns about `app-a#dev`
- if you keep waiting, `app-a` exits on its own and no second `Ctrl+C` is needed

## Scenario 4: multiple stubborn apps

```sh
APP_A_MODE=stubborn APP_B_MODE=stubborn APP_C_MODE=graceful \
  ../../../target/debug/turbo run dev --filter=app-a --filter=app-b --filter=app-c
```

Press `Ctrl+C`, wait about 3 seconds, then press `Ctrl+C` again.

Expected:

- `app-c` exits cleanly
- the slow-shutdown warning names `app-a#dev, app-b#dev`
- second `Ctrl+C` prints `^C - Force killing Turborepo tasks: app-a#dev, app-b#dev`

## Scenario 5: non-interactive mixed shutdown

```sh
APP_A_MODE=graceful APP_B_MODE=stubborn APP_C_MODE=slow \
  ../../../target/debug/turbo run dev --filter=app-a --filter=app-b --filter=app-c \
  </dev/null
```

Expected:

- Turbo prints `Shutting down Turborepo tasks...`
- after 3 seconds, it prints the slow-task snapshot
  `app-b#dev, app-c#dev`
- then it prints `Shutting down forcibly in 7s...`
- then it prints `Graceful shutdown timed out. Force killing Turborepo tasks: app-b#dev`
