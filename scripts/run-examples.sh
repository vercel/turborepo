#!/bin/bash

set -e


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
    rm -rf .git
}

function setup_git { 
   git init .
   git add . 
   git commit -m "Initial commit"      
}

for folder in examples/* ; do
  if [ -f "$folder/package.json" ]; then
    if [ "$folder" != "examples/with-pnpm" ]; then
      cd $folder 
      cleanup
      setup_git
      
      echo "-------------------------------------------------------"
      echo "$folder: yarn install"
      echo "-------------------------------------------------------"
      yarn install
      
      echo "-------------------------------------------------------"
      echo "$folder: yarn build lint"
      echo "-------------------------------------------------------"
      yarn build lint
      
      echo "-------------------------------------------------------"
      echo "$folder: yarn build again"
      echo "-------------------------------------------------------"
      yarn build lint
                
      echo "-------------------------------------------------------"    
      echo "$folder: SUCCESSFUL"
      echo "-------------------------------------------------------"    
      
      cleanup
      setup_git

      echo "-------------------------------------------------------"
      echo "$folder: npm install"
      echo "-------------------------------------------------------"
      npm install
      
      echo "-------------------------------------------------------"
      echo "$folder: npm build lint"
      echo "-------------------------------------------------------"
      npm run build lint
      
      echo "-------------------------------------------------------"
      echo "$folder: npm build lint again"
      echo "-------------------------------------------------------"
      npm run build lint
                
      echo "-------------------------------------------------------"    
      echo "$folder: SUCCESSFUL"
      echo "-------------------------------------------------------"    
      
      cleanup
      cd ../..
    fi
  fi
done;

# for folder in examples/* ; do
#   if [[ -f "$folder/package.json" ]]; then
#     if [[ -f $folder != "with-pnpm" ]]; then
#       cd $folder 
#       rm -rf node_modules
#       rm -rf **/node_modules/**
#       rm -rf **/.next
#       rm -rf **/dist
#       rm -rf **/build
#       rm -rf **/.turbo
#       rm -rf yarn.lock
#       rm -rf .git
#       git init .
#       git add . 
#       git commit -m "Initial commit"
#       echo "-------------------------------------------------------"
#       echo "$folder: yarn install"
#       yarn install
#       echo "$folder: yarn build lint"
#       yarn build lint
#       echo "$folder: yarn build again"
#       yarn build lint
#       echo "$folder: successful"
#       echo "-------------------------------------------------------"    
#       cd ../..
#     fi
#   fi
# done;

# if [ -f ".eslintrc.js.bak" ]; then
#   mv .eslintrc.js.bak .eslintrc.js 
# fi

# if [[ ! -z $(git status -s) ]];then
#   echo "Detected changes"
#   git status
#   exit 1
# fi