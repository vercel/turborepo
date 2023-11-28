#!/bin/bash

TARGET_DIR=$1
VERSION=$2

FILES=$(find $TARGET_DIR -type f -name package.json)

for FILE in $FILES
do
  rm $FILE
  echo "{ \"version\": \"$VERSION\" }" > $FILE
done
