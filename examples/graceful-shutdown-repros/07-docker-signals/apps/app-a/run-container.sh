#!/usr/bin/env bash
set -euo pipefail

if ! command -v docker >/dev/null 2>&1; then
  echo "docker CLI is required for this repro" >&2
  exit 1
fi

container_name="turbo-graceful-shutdown-$(id -u)-app-a"
image="${DOCKER_IMAGE:-alpine:3.20}"
mode="${CONTAINER_MODE:-graceful}"
host_dir="$(pwd -P)"

rm -f events.log ready pid sigint.txt sigterm.txt
docker rm -f "$container_name" >/dev/null 2>&1 || true

docker run --rm \
  --name "$container_name" \
  --sig-proxy=true \
  -e APP_NAME="app-a" \
  -e APP_DIR="/workspace" \
  -e CONTAINER_MODE="$mode" \
  -v "$host_dir:/workspace" \
  -w /workspace \
  "$image" \
  sh ./container-entrypoint.sh
