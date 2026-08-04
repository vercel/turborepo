#!/usr/bin/env bash
# Reproduces the runtime-visibility matrix for libc-getenv interposition.
# Requires: gcc, and optionally node/bun/python3/go/cargo on PATH.
set -u
cd "$(dirname "$0")"
WORK=$(mktemp -d)
trap 'rm -rf "$WORK"' EXIT

gcc -shared -fPIC -O2 -o "$WORK/libenvaudit.so" shim.c -ldl
gcc -O2 -o "$WORK/reader_dynamic" readers/reader.c
gcc -O2 -static -o "$WORK/reader_static" readers/reader.c 2>/dev/null || true
command -v go >/dev/null && CGO_ENABLED=0 go build -o "$WORK/reader_go" readers/reader.go
command -v rustc >/dev/null && rustc -O -o "$WORK/reader_rust" readers/reader.rs

run_case() {
  local name="$1"; shift
  local log="$WORK/log_$name.tsv"
  rm -f "$log"
  MY_SECRET_TOKEN=hunter2 LD_PRELOAD="$WORK/libenvaudit.so" ENV_AUDIT_OUT="$log" "$@" >/dev/null 2>&1
  local total=0; [ -f "$log" ] && total=$(wc -l <"$log")
  if [ -f "$log" ] && grep -q MY_SECRET_TOKEN "$log"; then
    echo "$name: CAUGHT (of $total getenv calls logged)"
  else
    echo "$name: MISSED (of $total getenv calls logged)"
  fi
}

run_case c_dynamic "$WORK/reader_dynamic"
[ -x "$WORK/reader_static" ] && run_case c_static "$WORK/reader_static"
[ -x "$WORK/reader_go" ] && run_case go_nocgo "$WORK/reader_go"
[ -x "$WORK/reader_rust" ] && run_case rust "$WORK/reader_rust"
command -v node >/dev/null && run_case node node -e 'process.env.MY_SECRET_TOKEN'
command -v bun >/dev/null && run_case bun bun -e 'process.env.MY_SECRET_TOKEN'
command -v python3 >/dev/null && run_case python3 python3 -c 'import os; os.environ.get("MY_SECRET_TOKEN")'
run_case bash bash -c 'echo "$MY_SECRET_TOKEN"'
run_case sh_dash sh -c 'echo "$MY_SECRET_TOKEN"'
