#!/bin/bash

if [ "$1" = "" ]; then
  .cram_env/bin/prysk --shell="$(which bash)" tests
else
  .cram_env/bin/prysk --shell="$(which bash)" "tests/$1"
fi
