#!/usr/bin/env bash

# generated changelog depends on the tag type
# only SEMVER tags are accounted
# there are two type of tag distinguished by minor part of the semver:
#   - even number: mainnet
#   - odd number: testnet
# net detection is done in section s1

# there are two type release notes generated
#   - prerelease: changelog between current and nearest lower prerelease (or previous release)
#     for example current tag v0.1.1-rc.10 and previous was v0.1.1-rc.9, so changelog is generated between
#   - release: changelog between current and previous release tags
# mainnet status is taken care as well. if current tag is edgenet (e.g. v0.1.1-rc.10) it
# will be generated to edgenet changes only

PATH=$PATH:$(pwd)/.cache/bin
export PATH=$PATH

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" >/dev/null 2>&1 && pwd)"

if [[ $# -ne 2 ]]; then
	echo "illegal number of parameters"
	exit 1
fi

to_tag=$1

version_rel="^[v|V]?(0|[1-9][0-9]*)\\.(0|[1-9][0-9]*)\\.?(0|[1-9][0-9]*)?$"
version_prerel="^[v|V]?(0|[1-9][0-9]*)\\.(0|[1-9][0-9]*)\\.(0|[1-9][0-9]*)(\\-[0-9A-Za-z-]+(\\.[0-9A-Za-z-]+)*)?(\\+[0-9A-Za-z-]+(\\.[0-9A-Za-z-]+)*)?$"

if [[ -z $("${SCRIPT_DIR}"/semver.sh get prerel "$to_tag") ]]; then
	tag_regexp=$version_rel
else
	tag_regexp=$version_prerel
fi

query_string="$to_tag"

git-chglog --config .chglog/config.yaml --tag-filter-pattern="$tag_regexp" --output "$2" "$query_string"
