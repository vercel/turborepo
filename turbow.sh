#!/bin/bash

cd cli && go build ./cmd/turbo && cd ..;
./cli/turbo $@
