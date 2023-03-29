#!/bin/bash

cat "$1" | jq ".tasks | map(select(.taskId == \"$2#build\")) | .[0]"
