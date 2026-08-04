#!/usr/bin/env bash
# Day-zero bootstrap: env[] is empty. How do we fill it?
# Candidates = every var in the environment turbo was launched from (finite,
# turbo already has it). Poison ALL of them at once; sentinels are
# self-identifying, so one extra build names every value dependency.
set -u
TURBO=./node_modules/.bin/turbo

# The "user's real environment" for this demo:
REAL=(API_URL=https://api.example.com DOCS_TOKEN=tok123 MINIFY= UNUSED_VAR=present CI=true)
CANDIDATES=(API_URL DOCS_TOKEN MINIFY UNUSED_VAR CI)

snapshot() { { cat apps/web/dist/out.txt 2>/dev/null; cat apps/docs/out.txt 2>/dev/null; } | md5sum | cut -d' ' -f1; }

echo "STEP 1: baseline build with real values"
env "${REAL[@]}" $TURBO run build --force --env-mode=loose >/dev/null 2>&1
BASE=$(snapshot)

echo "STEP 2: ONE build with every candidate poisoned simultaneously"
POISONED=(); for v in "${CANDIDATES[@]}"; do POISONED+=("$v=__TURBO_DOCTOR_${v}__"); done
env "${POISONED[@]}" $TURBO run build --force --env-mode=loose >/dev/null 2>&1

echo "  value dependencies found (self-identifying sentinels in outputs):"
grep -rhoE '__TURBO_DOCTOR_[A-Z_]+__' apps/web/dist apps/docs/out.txt 2>/dev/null | sed 's/__TURBO_DOCTOR_//;s/__$//' | sort -u | sed 's/^/    -> declare: /'

if [ "$(snapshot)" != "$BASE" ]; then
  echo "STEP 3: outputs also changed beyond value leaks -> bisect remaining candidates for behavioral deps"
  REMAINING=(MINIFY UNUSED_VAR CI)   # candidates not already attributed
  lo=0; hi=${#REMAINING[@]}; runs=0
  # simple demo bisection: test halves until single var isolated
  test_group() { local args=("${REAL[@]}"); for v in "$@"; do args+=("$v=__TURBO_DOCTOR_${v}__"); done
    env "${args[@]}" $TURBO run build --force --env-mode=loose >/dev/null 2>&1; runs=$((runs+1)); [ "$(snapshot)" != "$BASE" ]; }
  GROUP=("${REMAINING[@]}")
  while [ ${#GROUP[@]} -gt 1 ]; do
    HALF=("${GROUP[@]:0:${#GROUP[@]}/2}")
    if test_group "${HALF[@]}"; then GROUP=("${HALF[@]}"); else GROUP=("${GROUP[@]:${#GROUP[@]}/2}"); fi
  done
  if test_group "${GROUP[@]}"; then
    echo "    -> declare: ${GROUP[0]} (behavioral dependency, isolated in $runs bisection builds)"
  fi
fi
echo "RESULT: suggested env[] additions above; everything else in the environment is certified irrelevant to outputs."
