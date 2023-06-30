#!/bin/bash

std_dir="$(dirname "$0")"

infer_package_name() {
	# Convert the filename $1 to a package name. We munge the name as follows:
	#
	# 1. strip off the capnp file extension and dirname
	# 2. remove dashes
	# 3. convert '+' to 'x'. This is really just for c++.capnp, but it's not
	#    any easier to special case it.
	printf '%s' "$(basename $1)" | sed 's/\.capnp$// ; s/-//g ; s/+/x/g'
}

gen_annotated_schema() {
	# Copy the schema from file "$2" to the std/capnp directory, and add
	# appropriate $Go annotations. "$1" is the root of the input directory,
	# which is used to determine how much prefix to chop off.
	indir="$1"
	infile="$2"

	base=$(echo -n "$infile" | sed -e "s@$indir@@")
	outdir="$std_dir/capnp"
	outfile="$outdir/$base"
	package_name="$(infer_package_name "$outfile")"
	[ -d "$(dirname $outfile)" ] || mkdir -p "$(dirname $outfile)"
	cat "$infile" - > "$outfile" << EOF
using Go = import "/go.capnp";
\$Go.package("$package_name");
\$Go.import("capnproto.org/go/capnp/v3/std/capnp/$package_name");
EOF
}

find_capnp_files() {
	find "$1" -type f -name '*.capnp'
}

gen_go_src() {
	# Generate go source code from the schema file $1. Create the package
	# directory if necessary.
	file="$1"
        filedir="$(dirname "$file")"
	package_name="$(infer_package_name "$file")"
	mkdir -p "$filedir/$package_name" && \
        	capnp compile --no-standard-import -I"$std_dir" -ogo:"$filedir/$package_name" --src-prefix="$filedir" "$file"
}

usage() {
	echo "Usage:"
	echo ""
	echo "    $0 import <path/to/capnp/c++/src/capnp>"
	echo "    $0 patch      # Apply necessary patches to schema"
	echo "    $0 compile    # Generate go source files"
	echo "    $0 clean-go   # Remove go source files"
	echo "    $0 clean-all  # Remove go source files and imported schemas"
}

# do_* implements the corresponding subcommand described in usage's output.
do_import() {
	input_dir="$1"
	for file in $(find_capnp_files "$input_dir"); do
		gen_annotated_schema "$input_dir" "$file" || return 1
	done
}

do_patch() {
	cd "$std_dir" && patch -p1 < fixups.patch
}

do_compile() {
	for file in $(find_capnp_files "$std_dir"); do
		gen_go_src "$file" || return 1
	done
}

do_clean_go() {
	find "$std_dir" -name '*.capnp.go' -delete
	find "$std_dir" -type d -empty -delete
}

do_clean_all() {
	do_clean_go
	find "$std_dir/capnp" -name '*.capnp' -delete
}

eq_or_usage() {
	# If "$1" is not equal to "$2", call usage and exit.
	if [ ! $1 = $2 ] ; then
		usage
		exit 1
	fi
}

case "$1" in
	import)    eq_or_usage $# 2; do_import "$2" ;;
	patch) eq_or_usage $# 1; do_patch ;;
	compile)   eq_or_usage $# 1; do_compile ;;
	clean-go)  eq_or_usage $# 1; do_clean_go ;;
	clean-all) eq_or_usage $# 1; do_clean_all ;;
	*) usage; exit 1 ;;
esac
