#!/bin/bash
source "$(dirname "$0")/lib.sh"
node "$(dirname "$0")/gen.js" "$1"
