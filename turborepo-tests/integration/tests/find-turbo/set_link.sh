#!/bin/bash

TARGET_DIR=$1
TURBO_PATH=$2

if [[ "$OSTYPE" == "msys" ]]; then
  FILES=$(find $TARGET_DIR -type f -name turbo.exe)
else
  FILES=$(find $TARGET_DIR -type f -name turbo)
fi

for FILE in $FILES
do
  rm $FILE
  ln -s $TURBO_PATH $FILE
done
