#!/bin/bash

TARGET_DIR=$1
TURBO_PATH=$2

FILES=$(find $TARGET_DIR -type f -name turbo)

for FILE in $FILES
do
  rm $FILE
  ln -s $TURBO_PATH $FILE
done
