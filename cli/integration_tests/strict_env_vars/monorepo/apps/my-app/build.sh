#!/bin/bash

pathset="no"
shellset="no"
sysrootset="no"

if [ ! -z "$PATH" ]; then
  pathset="yes"
fi

if [ ! -z "$SHELL" ]; then
  shellset="yes"
fi

if [ ! -z "$SYSTEMROOT" ]; then
  sysrootset="yes"
fi

{
  echo -n "globalpt: '$GLOBAL_VAR_PT', "
  echo -n "localpt: '$LOCAL_VAR_PT', "
  echo -n "globaldep: '$GLOBAL_VAR_DEP', "
  echo -n "localdep: '$LOCAL_VAR_DEP', "
  echo -n "other: '$OTHER_VAR', "
  echo -n "sysroot set: '$sysrootset', "
  echo -n "path set: '$pathset', "
  echo "shell set: '$shellset'"
} > out.txt
