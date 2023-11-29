#!/bin/bash

set -e

if [ $# != 1 ]; then
  echo "Missing version"
  exit 1
fi
TARGET=$1

# Note: BASH_SOURCE[0] is a _relative_ path, so grab
# the full realpath right away before manipulating.
SCRIPT_FILE="$( realpath "${BASH_SOURCE[0]}" )"
PKG_ROOT=$(realpath "${SCRIPT_FILE}/../..")
JS_PACKAGE_JSON="${PKG_ROOT}/js/package.json"

echo "Version: ${TARGET}"
for dir in "${PKG_ROOT}"/npm/*; do
  dep="@turbo/repository-$(basename "${dir}")"
  # bump the native dependency version
  cd "${dir}" && pnpm version "${TARGET}" --allow-same-version

  # bump the optional dependency listed in the metapackage
  update_dep=$(jq ".optionalDependencies.\"${dep}\" = \$newVal" --arg newVal "${TARGET}" "${JS_PACKAGE_JSON}")
  echo "${update_dep}" > "${JS_PACKAGE_JSON}"
done

#finally, bump the top-level version of the metapackage
cd "${PKG_ROOT}/js" && pnpm version "${TARGET}" --allow-same-version
