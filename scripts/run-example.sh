#!/bin/bash

set -e

echo "=> Running examples..."
# echo "=> Building turbo from source..."
# cd cli && CGO_ENABLED=0 go build ./cmd/turbo/... && cd ..;
export TURBO_BINARY_PATH=$(pwd)/target/debug/turbo
export TURBO_VERSION=$(head -n 1 $(pwd)/version.txt)
export TURBO_TAG=$(cat $(pwd)/version.txt | sed -n '2 p')
export folder=$1
export pkgManager=$2
echo "=> Binary path: TURBO_BINARY_PATH=$TURBO_BINARY_PATH"
echo "=> Local Turbo Version: TURBO_VERSION=$TURBO_VERSION"
echo "=> Moving our own eslint settings out of the way..."
echo "=> Actually running examples for real..."

if [ -f ".eslintrc.js" ]; then
  mv .eslintrc.js .eslintrc.js.bak
fi

function cleanup {
  rm -rf node_modules
  rm -rf apps/*/node_modules
  rm -rf apps/*/.next
  rm -rf apps/*/.turbo
  rm -rf packages/*/node_modules
  rm -rf packages/*/.next
  rm -rf packages/*/.turbo
  rm -rf *.log
}

function setup_git {
  echo "=> Setting up git..."
  rm -rf .git
  mkdir .git
  touch .git/config
  echo "[user]" >> .git/config
  echo "  name = GitHub Actions" >> .git/config
  echo "  email = actions@users.noreply.github.com" >> .git/config
  echo "" >> .git/config
  echo "[init]" >> .git/config
  echo "  defaultBranch = main" >> .git/config
  git init . -q
  git add .
  git commit -m "Initial commit"
}

function run_npm {
  cat package.json | jq '.packageManager = "npm@8.1.2"' | sponge package.json
  if [ "$TURBO_TAG" == "canary" ]; then
    cat package.json | jq '.devDependencies.turbo = "canary"' | sponge package.json
  fi

  echo "======================================================="
  echo "=> $folder: move lockfile"
  echo "======================================================="
  [ ! -f yarn.lock ] || mv yarn.lock yarn.lock.bak
  [ ! -f pnpm-lock.yaml ] || mv pnpm-lock.yaml pnpm-lock.yaml.bak
  [ ! -f package-lock.json ] || mv package-lock.json package-lock.json.bak

  echo "======================================================="
  echo "=> $folder: npm install"
  echo "======================================================="
  npm install --force

  echo "======================================================="
  echo "=> $folder: npm build lint"
  echo "======================================================="
  if [ $folder == "non-monorepo" ]; then
    npx turbo build lint
  else
    npm run build lint
  fi

  echo "======================================================="
  echo "=> $folder: npm build lint again"
  echo "======================================================="
  if [ $folder == "non-monorepo" ]; then
    npx turbo build lint
  else
    npm run build lint
  fi

  # restore lockfile
  rm -rf package-lock.json
  [ ! -f yarn.lock.bak ] || mv yarn.lock.bak yarn.lock
  [ ! -f pnpm-lock.yaml.bak ] || mv pnpm-lock.yaml.bak pnpm-lock.yaml
  [ ! -f package-lock.json.bak ] || mv package-lock.json.bak package-lock.json

  echo "======================================================="
  echo "=> $folder: npm SUCCESSFUL"
  echo "======================================================="
}

function run_pnpm {
  cat package.json | jq '.packageManager = "pnpm@6.26.1"' | sponge package.json
  if [ "$TURBO_TAG" == "canary" ]; then
    cat package.json | jq '.devDependencies.turbo = "canary"' | sponge package.json
  fi

  echo "======================================================="
  echo "=> $folder: move lockfile"
  echo "======================================================="
  [ ! -f yarn.lock ] || mv yarn.lock yarn.lock.bak
  [ ! -f pnpm-lock.yaml ] || mv pnpm-lock.yaml pnpm-lock.yaml.bak
  [ ! -f package-lock.json ] || mv package-lock.json package-lock.json.bak

  echo "======================================================="
  echo "=> $folder: pnpm install"
  echo "======================================================="
  pnpm install

  echo "======================================================="
  echo "=> $folder: pnpm build lint"
  echo "======================================================="
  if [ $folder == "non-monorepo" ]; then
    pnpm turbo build lint
  else
    pnpm run build lint
  fi

  echo "======================================================="
  echo "=> $folder: pnpm build lint again"
  echo "======================================================="
  if [ $folder == "non-monorepo" ]; then
    pnpm turbo build lint
  else
    pnpm run build lint
  fi

  # restore lockfile
  rm -rf pnpm-lock.yaml
  [ ! -f yarn.lock.bak ] || mv yarn.lock.bak yarn.lock
  [ ! -f pnpm-lock.yaml.bak ] || mv pnpm-lock.yaml.bak pnpm-lock.yaml
  [ ! -f package-lock.json.bak ] || mv package-lock.json.bak package-lock.json

  echo "======================================================="
  echo "=> $folder: pnpm SUCCESSFUL"
  echo "======================================================="
}

function run_yarn {
  cat package.json | jq '.packageManager = "yarn@1.22.17"' | sponge package.json
  if [ "$TURBO_TAG" == "canary" ]; then
    cat package.json | jq '.devDependencies.turbo = "canary"' | sponge package.json
  fi

  echo "======================================================="
  echo "=> $folder: move lockfile"
  echo "======================================================="
  [ ! -f yarn.lock ] || mv yarn.lock yarn.lock.bak
  [ ! -f pnpm-lock.yaml ] || mv pnpm-lock.yaml pnpm-lock.yaml.bak
  [ ! -f package-lock.json ] || mv package-lock.json package-lock.json.bak


  echo "======================================================="
  echo "=> $folder: yarn install"
  echo "======================================================="
  yarn install

  echo "======================================================="
  echo "=> $folder: yarn build lint"
  echo "======================================================="
  if [ $folder == "non-monorepo" ]; then
    yarn turbo build lint
  else
    yarn build lint
  fi

  echo "======================================================="
  echo "=> $folder: yarn build again"
  echo "======================================================="
  if [ $folder == "non-monorepo" ]; then

    yarn turbo build lint
  else
    yarn build lint
  fi

    # restore lockfile
  rm -rf yarn.lock
  [ ! -f yarn.lock.bak ] || mv yarn.lock.bak yarn.lock
  [ ! -f pnpm-lock.yaml.bak ] || mv pnpm-lock.yaml.bak pnpm-lock.yaml
  [ ! -f package-lock.json.bak ] || mv package-lock.json.bak package-lock.json

  echo "======================================================="
  echo "=> $folder: yarn SUCCESSFUL"
  echo "======================================================="
}


hasRun=0
if [ -f "examples/$folder/package.json" ]; then
  cd "examples/$folder"
  echo "======================================================="
  echo "=> checking $folder "
  echo "======================================================="
  cleanup
  setup_git
  # save the default packageManager
  packageManager=$(cat 'package.json' | jq '.packageManager')
  if [ "$pkgManager" == "npm" ]; then
    run_npm
  elif [ "$pkgManager" == "pnpm" ]; then
    run_pnpm
  elif [ "$pkgManager" == "yarn" ]; then
    run_yarn
  else
    echo "Unknown package manager ${folder}"
    exit 2
  fi
  hasRun=1

  # reset to the default packageManager
  cat package.json | jq ".packageManager = ${packageManager}" | sponge package.json
  if [ "$TURBO_TAG" == "canary" ]; then
    cat package.json | jq '.devDependencies.turbo = "latest"' | sponge package.json
  fi

  cleanup

  cd ../..
fi

if [ -f ".eslintrc.js.bak" ]; then
  mv .eslintrc.js.bak .eslintrc.js
fi

if [[ ! -z $(git status -s) ]];then
  echo "Detected changes"
  git status
  exit 1
fi

if [ $hasRun -eq 0 ]; then
  echo "Did not run any examples"
  exit 2
fi
