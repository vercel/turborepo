#!/bin/bash

# relative
TURBO_CACHE_DIR=tmp/things ./target/debug/turbo  config | jq .cacheDir

# relative
./target/debug/turbo --cache-dir tmp/things config | jq .cacheDir

# absolute
./target/debug/turbo config | jq .cacheDir

# absolute
TURBO_CACHE_DIR=/tmp/things ./target/debug/turbo  config | jq .cacheDir

# absolute
./target/debug/turbo --cache-dir /tmp/things config | jq .cacheDir
