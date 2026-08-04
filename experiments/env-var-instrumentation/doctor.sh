#!/usr/bin/env bash
# "turbo env doctor" prototype: black-box detection of which undeclared env
# vars affect task outputs. For each candidate var, rerun the task with a
# sentinel value; the var matters if (a) the sentinel string leaks into any
# output (value dependency), or (b) outputs differ from baseline (behavioral
# dependency). Language-agnostic: works on shell, Go, anything.
set -u
CANDIDATES=(API_URL DOCS_TOKEN MINIFY UNUSED_VAR)
TURBO=./node_modules/.bin/turbo

snapshot() { # capture all task outputs into a hashable form
  { cat apps/web/dist/out.txt 2>/dev/null; cat apps/docs/out.txt 2>/dev/null; } | md5sum | cut -d' ' -f1
}

run_build() { # run with given env assignments (loose mode = doctor passes everything)
  env "$@" API_URL="${API_URL_V-https://api.example.com}" \
    $TURBO run build --force --env-mode=loose >/dev/null 2>&1
}

# Baseline: real values
API_URL=https://api.example.com DOCS_TOKEN=tok123 MINIFY= UNUSED_VAR=present \
  $TURBO run build --force --env-mode=loose >/dev/null 2>&1
BASE=$(snapshot)

for VAR in "${CANDIDATES[@]}"; do
  SENTINEL="__TURBO_DOCTOR_${VAR}__"
  env API_URL=https://api.example.com DOCS_TOKEN=tok123 MINIFY= UNUSED_VAR=present \
      "$VAR=$SENTINEL" \
      $TURBO run build --force --env-mode=loose >/dev/null 2>&1
  LEAK=$(grep -rls "$SENTINEL" apps/web/dist apps/docs/out.txt 2>/dev/null | head -1)
  NOW=$(snapshot)
  if [ -n "$LEAK" ]; then
    echo "$VAR: AFFECTS OUTPUT (sentinel value found in $LEAK) -> declare in env[]"
  elif [ "$NOW" != "$BASE" ]; then
    echo "$VAR: AFFECTS OUTPUT (behavioral: outputs changed, no value leak) -> declare in env[]"
  else
    echo "$VAR: no effect on outputs -> safe to omit"
  fi
done
