#!/bin/bash

# relative
TURBO_CACHE_DIR=tmp/things ./target/debug/turbo  config

# relative
./target/debug/turbo --cache-dir tmp/things config

# absolute
./target/debug/turbo config

# absolute
TURBO_CACHE_DIR=/tmp/things ./target/debug/turbo  config

# absolute
./target/debug/turbo --cache-dir /tmp/things config
