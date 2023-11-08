#!/bin/bash

function getSummaryTask() {
  cat "$1" | jq ".tasks | map(select(.task == \"$2\")) | .[0]"
}

function getSummaryTaskId() {
  cat "$1" | jq ".tasks | map(select(.taskId == \"$2\")) | .[0]"
}
