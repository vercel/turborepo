#!/usr/bin/env bash

DIR=${TESTDIR}
while [ $(basename $DIR) != "cli" ]; do
  DIR=$(dirname $DIR)
done
SHIM=${DIR}/../target/debug/turbo
VERSION=${DIR}/../version.txt
