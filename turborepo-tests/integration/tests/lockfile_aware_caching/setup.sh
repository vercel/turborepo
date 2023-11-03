#!/bin/bash

THIS_SCRIPT=$(dirname ${BASH_SOURCE[0]})

FIXTURE_DIR=$1

cp -a ${THIS_SCRIPT}/${FIXTURE_DIR}/. ${PWD}/

# echo ".turbo" >> ${PWD}/.gitignore
# echo "node_modules" >> ${PWD}/.gitignore

git ${GIT_ARGS} add .
git ${GIT_ARGS} commit -m "Add lockfiles" --quiet
