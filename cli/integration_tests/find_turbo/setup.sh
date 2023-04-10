#!/bin/bash

SCRIPT_DIR=$(dirname ${BASH_SOURCE[0]})
TARGET_DIR=$1
FIXTURE_DIR=$2

cp -a ${SCRIPT_DIR}/../_fixtures/find_turbo/$FIXTURE_DIR/. ${TARGET_DIR}/
