#!/bin/bash

cd /Users/qua/vercel/next-evals-oss

for eval in evals/*/input; do
  eval_name=$(basename $(dirname $eval))
  cd "$eval"
  if pnpm build-only > /dev/null 2>&1 && pnpm lint > /dev/null 2>&1; then
    echo "✅ $eval_name"
  else
    echo "❌ $eval_name"
  fi
  cd /Users/qua/vercel/next-evals-oss
done
