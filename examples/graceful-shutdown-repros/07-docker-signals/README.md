# 07 Docker Signals

Tests how a long-running `docker run` task behaves when Turbo shuts down.

This fixture runs an attached container from `alpine:3.20`. The container
writes markers into `apps/app-a/` so you can inspect what the container saw,
even if the `docker run` client exits first.

Requirements:

- Docker Desktop / Docker Engine running locally
- the first run may pull `alpine:3.20`

Run from this directory with the locally built binary:

```sh
CONTAINER_MODE=graceful ../../../target/debug/turbo run dev --filter=app-a
```

The container name used by this fixture is:

```sh
container_name="turbo-graceful-shutdown-$(id -u)-app-a"
```

## Scenario 1: graceful Ctrl+C

```sh
CONTAINER_MODE=graceful ../../../target/debug/turbo run dev --filter=app-a
```

Press `Ctrl+C` once.

Expected:

- task output includes `app-a container received SIGINT`
- then `app-a container exiting after SIGINT`
- `apps/app-a/sigint.txt` exists
- `docker ps --filter "name=$container_name"` shows no running container

## Scenario 2: stubborn container

```sh
CONTAINER_MODE=stubborn ../../../target/debug/turbo run dev --filter=app-a
```

Press `Ctrl+C`, wait about 3 seconds, then press `Ctrl+C` again.

Expected:

- the container logs `app-a container received SIGINT`
- then `app-a container ignoring SIGINT`
- Turbo eventually force-kills the `docker run` task
- the Docker container is still running, because the Docker daemon outlives the
  killed client process

Check:

```sh
container_name="turbo-graceful-shutdown-$(id -u)-app-a"
docker ps --filter "name=$container_name" --format '{{.Names}}'
```

If it is still running, clean it up:

```sh
docker rm -f "$container_name"
```

## Scenario 3: kill Turbo with SIGKILL

```sh
container_name="turbo-graceful-shutdown-$(id -u)-app-a"
CONTAINER_MODE=graceful ../../../target/debug/turbo run dev --filter=app-a &
turbo_pid=$!
while [ ! -f apps/app-a/ready ]; do sleep 0.1; done
kill -9 "$turbo_pid"
sleep 2
docker ps --filter "name=$container_name" --format '{{.Names}}'
```

Expected:

- Turbo dies immediately
- the attached `docker run` client dies with it
- the Docker daemon sends `SIGTERM` to the graceful container, so it should log
  `received SIGTERM` and exit shortly after
- `docker ps --filter "name=$container_name"` may briefly show the container
  while it is shutting down; rerun the command after another second if needed

## Scenario 4: slow container

```sh
CONTAINER_MODE=slow ../../../target/debug/turbo run dev --filter=app-a
```

Press `Ctrl+C` once.

Expected:

- the container logs `app-a container taking awhile to exit after SIGINT`
- after about 3 seconds Turbo warns that `app-a#dev` is taking awhile to shut down
- if you keep waiting, the container exits on its own without a second `Ctrl+C`

## Useful files

- `apps/app-a/events.log`
- `apps/app-a/pid`
- `apps/app-a/ready`
- `apps/app-a/sigint.txt`
- `apps/app-a/sigterm.txt`
