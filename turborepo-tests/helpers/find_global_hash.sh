#!/bin/bash

# This script greps stdin (i.e. what's piped to it)
# splits it by "=" and prints the second value.
# it's intendted to get the global hash from a debug log that looks like this:
# 2023-04-06T04:28:19.599Z [DEBUG] turbo: global hash: value=a027dadc4dea675e
#
# Usage:
# turbo build -vv 2>&1 | "$TESTDIR/./find_global_hash.sh"
#
#
grep "global hash:" - | awk '{split($0,a,"="); print a[2]}'
