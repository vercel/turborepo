#!/usr/bin/env bash

# in akash even minor part of the tag indicates release belongs to the MAINNET
# using it as scripts simplifies debugging as well as portability
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" >/dev/null 2>&1 && pwd)"

if [[ $# -ne 1 ]]; then
	echo "illegal number of parameters"
	exit 1
fi

[[ -n $("${SCRIPT_DIR}"/semver.sh get prerel "$1") ]] && echo true || echo false
