#!/bin/sh

DIR=${TESTDIR}
while [ $(basename $DIR) != "cli" ]; do
  DIR=$(dirname $DIR)
done
TURBO=${DIR}/turbo
