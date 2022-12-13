#!/bin/bash

SCRIPT_DIR=$(dirname ${BASH_SOURCE[0]})
VERSION=$1
cp -a ${SCRIPT_DIR}/repo_with_local/. $(pwd)/
cat ${SCRIPT_DIR}/package_template.json | jq --argjson ver \"${VERSION}\" '{ name: .name, version: $ver}' > node_modules/turbo/package.json
