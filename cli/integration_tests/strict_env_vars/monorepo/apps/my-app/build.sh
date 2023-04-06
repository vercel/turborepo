#!/bin/bash

pathset="no"
sysrootset="no"

if [ ! -z "$PATH" ]; then
  pathset="yes"
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
  echo "path set: '$pathset'"
} > out.txt
