#!/bin/bash

# disable package manager update notifiers
export NO_UPDATE_NOTIFIER=1
if [ "$1" = "" ]; then
  .cram_env/bin/prysk --shell="$(which bash)" tests
else
  .cram_env/bin/prysk --shell="$(which bash)" "tests/$1"
fi
