#!/usr/bin/env bash
set -euo pipefail

mode=${1:-}

if [[ -z "${SCCACHE_DIR:-}" ]]; then
  echo "SCCACHE_DIR is required"
  exit 1
fi

if ! command -v rustc >/dev/null 2>&1; then
  echo "rustc is required to derive the sccache blob key"
  exit 1
fi

if ! command -v git >/dev/null 2>&1; then
  echo "git is required to derive the sccache blob key"
  exit 1
fi

blob_prefix=${SCCACHE_BLOB_PREFIX:-sccache}
blob_version=${SCCACHE_BLOB_VERSION:-v1}
runner_os=${RUNNER_OS:-unknown-os}
runner_arch=${RUNNER_ARCH:-unknown-arch}
rust_key=$(rustc -Vv | git hash-object --stdin | cut -c 1-16)
blob_path="${blob_prefix%/}/${blob_version}/${runner_os}-${runner_arch}/${rust_key}.tar.zst"
archive_path="${RUNNER_TEMP:-/tmp}/sccache-${runner_os}-${runner_arch}-${rust_key}.tar.zst"
vercel_cli_version=${VERCEL_CLI_VERSION:-54.1.0}

require_zstd() {
  if ! command -v zstd >/dev/null 2>&1; then
    echo "zstd is required for sccache archive restore/save"
    exit 1
  fi
}

validate_blob_base_url() {
  if [[ ! "${SCCACHE_BLOB_BASE_URL}" =~ ^https://[A-Za-z0-9.-]+\.public\.blob\.vercel-storage\.com/?$ ]]; then
    echo "SCCACHE_BLOB_BASE_URL must be an HTTPS public Vercel Blob URL"
    exit 1
  fi
}

if [[ -n "${GITHUB_ENV:-}" ]]; then
  echo "SCCACHE_BLOB_PATH=${blob_path}" >>"$GITHUB_ENV"
fi

restore() {
  mkdir -p "$SCCACHE_DIR"

  if [[ -z "${SCCACHE_BLOB_BASE_URL:-}" ]]; then
    echo "SCCACHE_BLOB_BASE_URL is not set; skipping sccache restore"
    return 0
  fi

  require_zstd
  validate_blob_base_url

  local url="${SCCACHE_BLOB_BASE_URL%/}/${blob_path}"
  echo "Restoring sccache archive from ${url}"

  local http_code
  http_code=$(curl \
    --location \
    --silent \
    --show-error \
    --retry 3 \
    --retry-all-errors \
    --connect-timeout 15 \
    --max-time 600 \
    --write-out "%{http_code}" \
    --output "$archive_path" \
    "$url" || true)

  if [[ "$http_code" != "200" ]]; then
    echo "No sccache archive restored; HTTP ${http_code:-unknown}"
    rm -f "$archive_path"
    return 0
  fi

  if [[ ! -s "$archive_path" ]]; then
    echo "Downloaded sccache archive is empty; skipping restore"
    rm -f "$archive_path"
    return 0
  fi

  zstd -d -c "$archive_path" | tar -tf - | while IFS= read -r entry; do
    normalized_entry=${entry//\\//}
    case "$normalized_entry" in
      "" | /* | [A-Za-z]:* | .. | ../* | */.. | */../*)
        echo "Refusing unsafe sccache archive entry: $entry"
        exit 1
        ;;
    esac
  done

  zstd -d -c "$archive_path" | tar -tvf - | while IFS= read -r entry; do
    case "${entry:0:1}" in
      - | d)
        ;;
      *)
        echo "Refusing non-file sccache archive entry: $entry"
        exit 1
        ;;
    esac
  done

  local restore_dir="${RUNNER_TEMP:-/tmp}/sccache-restore-${runner_os}-${runner_arch}-${rust_key}"
  rm -rf "$restore_dir"
  mkdir -p "$restore_dir"
  zstd -d -c "$archive_path" | tar -xf - -C "$restore_dir"
  rm -rf "$SCCACHE_DIR"
  mkdir -p "$SCCACHE_DIR"
  cp -a "$restore_dir/." "$SCCACHE_DIR/"
  rm -rf "$restore_dir"
  rm -f "$archive_path"
  echo "Restored sccache archive to $SCCACHE_DIR"
}

save() {
  local token=${SCCACHE_BLOB_READ_WRITE_TOKEN:-${BLOB_READ_WRITE_TOKEN:-}}
  if [[ -z "$token" ]]; then
    echo "No Vercel Blob read-write token is available; skipping sccache save"
    return 0
  fi

  if [[ ! -d "$SCCACHE_DIR" ]]; then
    echo "SCCACHE_DIR does not exist; skipping sccache save"
    return 0
  fi

  require_zstd

  sccache --show-stats || true
  sccache --stop-server || true

  tar -cf - -C "$SCCACHE_DIR" . | zstd -T0 -3 -f -o "$archive_path"

  if [[ ! -s "$archive_path" ]]; then
    echo "Created sccache archive is empty; skipping upload"
    rm -f "$archive_path"
    return 0
  fi

  export BLOB_READ_WRITE_TOKEN="$token"
  echo "Uploading sccache archive to Vercel Blob path ${blob_path}"
  npx --yes "vercel@${vercel_cli_version}" blob put "$archive_path" \
    --access public \
    --pathname "$blob_path" \
    --allow-overwrite \
    --cache-control-max-age 31536000
  rm -f "$archive_path"
}

case "$mode" in
  restore)
    restore
    ;;
  save)
    save
    ;;
  *)
    echo "Usage: $0 {restore|save}"
    exit 1
    ;;
esac
