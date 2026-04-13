#!/bin/sh
set -eu

app_name="${APP_NAME:-app-a}"
mode="${CONTAINER_MODE:-graceful}"
app_dir="${APP_DIR:-$(pwd)}"
events_file="$app_dir/events.log"
shutting_down=0

if [ ! -d "$app_dir" ]; then
  echo "expected app directory at '$app_dir', but it was missing" >&2
  exit 1
fi

append() {
  message="$1"
  echo "$message"
  printf '%s %s\n' "$(date -u +"%Y-%m-%dT%H:%M:%SZ")" "$message" >> "$events_file"
}

printf '%s\n' "$$" > "$app_dir/pid"
: > "$app_dir/ready"
append "$app_name container ready mode=$mode pid=$$"

handle_signal() {
  signal="$1"

  if [ "$shutting_down" -eq 1 ]; then
    return
  fi

  printf '%s\n' "$signal" > "$app_dir/$(printf '%s' "$signal" | tr 'A-Z' 'a-z').txt"
  append "$app_name container received $signal"

  case "$mode" in
    graceful)
      shutting_down=1
      sleep 1
      append "$app_name container exiting after $signal"
      exit 0
      ;;
    slow)
      shutting_down=1
      append "$app_name container taking awhile to exit after $signal"
      sleep 10
      append "$app_name container exiting after $signal"
      exit 0
      ;;
    stubborn)
      append "$app_name container ignoring $signal"
      ;;
    *)
      append "$app_name container exiting because mode '$mode' is unsupported"
      exit 1
      ;;
  esac
}

trap 'handle_signal SIGINT' INT
trap 'handle_signal SIGTERM' TERM

while :; do
  sleep 1
done
