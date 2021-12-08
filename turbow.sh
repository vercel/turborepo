#!/bin/bash

cd cli && go build ./cmd/turbo/... && cd ..;

UNAME=$(uname)

if [ "$UNAME" == "Linux" ] ; then
	./cli/turbo $@
elif [ "$UNAME" == "Darwin" ] ; then
	./cli/turbo $@
else
	./cli/turbo.exe $@
fi
