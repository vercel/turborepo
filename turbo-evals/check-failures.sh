#!/bin/bash

for eval in 021-avoid-fetch-in-effect 023-avoid-getserversideprops 026-no-serial-await 030-app-router-migration-hard 031-ai-sdk-migration-simple 035-ai-sdk-call-tools 036-ai-sdk-call-tools-multiple-steps; do
  echo "=== $eval ==="
  cd /Users/qua/vercel/next-evals-oss/evals/$eval/input
  pnpm build-only 2>&1 | grep -A 5 "Type error\|Error:" | head -10
  echo ""
done
