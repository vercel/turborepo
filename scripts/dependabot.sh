#!/usr/bin/env bash
set -e

# USAGE:
# git ls-remote https://github.com/vercel/turborepo.git | sed -n 's/.*\trefs\/heads\/dependabot\/npm_and_yarn/dependabot\/npm_and_yarn/p' | xargs -L 1 ./scripts/dependabot.sh

BRANCH=$1
echo "Updating $BRANCH"
git reset --hard
git fetch origin
git checkout --track "origin/$BRANCH"
git reset --hard "origin/$BRANCH"
git rebase "origin/main"
pnpm install
git add pnpm-lock.yaml
git commit -nm "Regenerate lockfile."
git push -f origin
git checkout main
git branch -D $BRANCH
