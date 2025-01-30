#!/bin/bash

set -e

# Note: BASH_SOURCE[0] is a _relative_ path, so grab
# the full realpath right away before manipulating.
SCRIPT_FILE="$( realpath "${BASH_SOURCE[0]}" )"
echo "SCRIPT_FILE: ${SCRIPT_FILE}"
PKG_ROOT=$(realpath "$(dirname "${SCRIPT_FILE}")/..")
echo "PKG_ROOT: ${PKG_ROOT}"
JS_PACKAGE_JSON="${PKG_ROOT}/js/package.json"
echo "JS_PACKAGE_JSON: ${JS_PACKAGE_JSON}"

if [ $# != 1 ]; then
  CURRENT_VERSION=$(jq -r .version "${JS_PACKAGE_JSON}")
  echo "Missing version, current version is $CURRENT_VERSION.  Please provide a new version as an argument to this script."
  exit 1
fi

TARGET=$1

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
