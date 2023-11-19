#!/usr/bin/env bash

if [[ "$OSTYPE" == "msys" ]]; then
    echo "Building stub turbo.exe for windows platform"
    g++ turbo.cpp -o turbo.exe
else
  echo "Skipping build for non-windows platform"
fi

