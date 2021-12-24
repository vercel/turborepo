#!/bin/bash

set -e

echo "=> Running examples..."
echo "=> Building turbo from source..."
cd cli && CGO_ENABLED=0 go build ./cmd/turbo/... && cd ..;
export TURBO_BINARY_PATH=$(pwd)/cli/turbo
echo "=> Binary path: TURBO_BINARY_PATH=$TURBO_BINARY_PATH"
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
    rm -rf yarn.lock
    rm -rf package-lock.json   
    rm -rf pnpm-lock.yaml
    rm -rf .git
    
}

function setup_git { 
   echo "=> Setting up git..."
   git init .
   git add . 
   git commit -m "Initial commit"      
}

for folder in examples/* ; do
  if [ -f "$folder/package.json" ]; then
    cd $folder 
    echo "======================================================="
    echo "=> checking $folder "
    echo "======================================================="
    if [ "$folder" != "examples/with-pnpm" ]; then
      
      cleanup
      setup_git
      
      echo "======================================================="
      echo "=> $folder: yarn install"
      echo "======================================================="
      yarn install
      
      echo "======================================================="
      echo "=> $folder: yarn build lint"
      echo "======================================================="
      yarn build lint
      
      echo "======================================================="
      echo "=> $folder: yarn build again"
      echo "======================================================="
      yarn build lint
                
      echo "======================================================="    
      echo "=> $folder: yarn SUCCESSFUL"
      echo "======================================================="    
      
      cleanup
      setup_git

      echo "======================================================="
      echo "=> $folder: npm install"
      echo "======================================================="
      npm install
      
      echo "======================================================="
      echo "=> $folder: npm build lint"
      echo "======================================================="
      npm run build lint
      
      echo "======================================================="
      echo "=> $folder: npm build lint again"
      echo "======================================================="
      npm run build lint
                
      echo "======================================================="    
      echo "=> $folder: npm SUCCESSFUL"
      echo "======================================================="    
    fi

    if [ "$folder" == "examples/with-pnpm" ]; then
      cleanup
      setup_git      
      echo "======================================================="
      echo "=> $folder: pnpm install"
      echo "======================================================="
      pnpm install
      
      echo "======================================================="
      echo "=> $folder: pnpm build lint"
      echo "======================================================="
      pnpm run build lint
      
      echo "======================================================="
      echo "=> $folder: pnpm build lint again"
      echo "======================================================="
      pnpm run build lint
                
      echo "======================================================="    
      echo "=> $folder: pnpm SUCCESSFUL"
      echo "======================================================="               
    fi
    

    cleanup

    cd ../..
  fi
done;


if [ -f ".eslintrc.js.bak" ]; then
  mv .eslintrc.js.bak .eslintrc.js 
fi

if [[ ! -z $(git status -s) ]];then
  echo "Detected changes"
  git status
  exit 1
fi