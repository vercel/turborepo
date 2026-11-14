#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"
pnpm exec buf generate
pnpm -w fmt-select "$ROOT/src/gen"
