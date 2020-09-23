#!/usr/bin/env bash

PROG=sysroot-rsync

USAGE="\
Usage:
  $PROG remote_host local_dir
  $PROG --help
Arguments:
  remove_host ssh host to sync from
  local_dir   local dir to sync to
Options:
  -v, --version   Print the version of this tool.
  -h, --help      Print this help message.
  -i              SSH key"

argn=$#

#DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" >/dev/null 2>&1 && pwd)"

function error() {
	echo -e "$1" >&2
	exit 1
}

function usage-help() {
	error "$USAGE"
}

ssh_key=""

exclude_list=()
include_list=()

exclude_list+=(--exclude "/bin")
exclude_list+=(--exclude "/boot")
exclude_list+=(--exclude "/boot*")
exclude_list+=(--exclude "/dev")
exclude_list+=(--exclude "/etc")
exclude_list+=(--exclude "/home")
exclude_list+=(--exclude "/lib/dhcpd")
exclude_list+=(--exclude "/lib/firmware")
exclude_list+=(--exclude "/lib/hdparm")
exclude_list+=(--exclude "/lib/ifupdown")
exclude_list+=(--exclude "/lib/modules")
exclude_list+=(--exclude "/lib/modprobe.d")
exclude_list+=(--exclude "/lib/modules-load.d")
exclude_list+=(--exclude "/lib/resolvconf")
exclude_list+=(--exclude "/lib/startpar")
exclude_list+=(--exclude "/lib/systemd")
exclude_list+=(--exclude "/lib/terminfo")
exclude_list+=(--exclude "/lib/udev")
exclude_list+=(--exclude "/lib/xtables")
exclude_list+=(--exclude "/lib/ssl/private")
exclude_list+=(--exclude "/lost+found")
exclude_list+=(--exclude "/media")
exclude_list+=(--exclude "/mnt")
exclude_list+=(--exclude "/proc")
exclude_list+=(--exclude "/root")
exclude_list+=(--exclude "/run")
exclude_list+=(--exclude "/sbin")
exclude_list+=(--exclude "/srv")
exclude_list+=(--exclude "/sys")
exclude_list+=(--exclude "/tmp")
exclude_list+=(--exclude "/usr/bin")
exclude_list+=(--exclude "/usr/games")
exclude_list+=(--exclude "/usr/sbin")
exclude_list+=(--exclude "/usr/share")
exclude_list+=(--exclude "/usr/src")
exclude_list+=(--exclude "/usr/local/bin")
exclude_list+=(--exclude "/usr/local/etc")
exclude_list+=(--exclude "/usr/local/games")
exclude_list+=(--exclude "/usr/local/man")
exclude_list+=(--exclude "/usr/local/sbin")
exclude_list+=(--exclude "/usr/local/share")
exclude_list+=(--exclude "/usr/local/src")
exclude_list+=(--exclude "/usr/lib/ssl/private")
exclude_list+=(--exclude "/var")
exclude_list+=(--exclude "/snap")
exclude_list+=(--exclude "*python*")

include_list+=(--include "*.a")
include_list+=(--include "*.so")
include_list+=(--include "*.so.*")
include_list+=(--include "*.h")
include_list+=(--include "*.hh")
include_list+=(--include "*.hpp")
include_list+=(--include "*.hxx")
include_list+=(--include "*.pc")
include_list+=(--include "/lib")
include_list+=(--include "/lib32")
include_list+=(--include "/lib64")
include_list+=(--include "/libx32")
include_list+=(--include "*/")

function do-sync() {
	from=$1
	to=$2

	args=()
	args+=(-a)
	args+=(-z)
	args+=(-m)
	args+=(-d)
	args+=(-h)

	if [[ -n $ssh_key ]]; then
		args+=("-e \"ssh -i $ssh_key\"")
	fi

	args+=(--keep-dirlinks)
	args+=("--info=progress2")
	args+=(--delete)
	args+=(--prune-empty-dirs)
	args+=(--sparse)
	args+=(--links)
	args+=(--copy-unsafe-links)
	args+=("${exclude_list[@]}")
	args+=("${include_list[@]}")
	args+=(--exclude "*")
	args+=("$from")
	args+=("$to")

	echo "${args[@]}"
	rsync "${args[@]}"

	exit $?
}

while getopts ":i:" opt; do
	# shellcheck disable=SC2220
	case ${opt} in
		i)
			ssh_key=$OPTARG
			;;
		:)
			echo "Invalid option: $OPTARG requires an argument" 1>&2
			usage-help
			;;
	esac
done

shift $((OPTIND -1))
argn=$((argn - $((OPTIND -1))))

case $1 in
	--help | -h)
		echo -e "$USAGE"
		exit 0
		;;
	*)
		if [[ $argn -ne 2 ]]; then
			echo "$argn"
			error "invalid arguments"
			usage-help
			exit 1
		fi

		do-sync "$1" "$2"
		;;
esac
