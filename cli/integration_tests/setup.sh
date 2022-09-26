#!/usr/bin/env bash

DIR=${TESTDIR}
while [ $(basename $DIR) != "cli" ]; do
  DIR=$(dirname $DIR)
done
TURBO=${DIR}/turbo
