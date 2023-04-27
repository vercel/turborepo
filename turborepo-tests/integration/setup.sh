#!/bin/bash

python3 -m venv .cram_env
.cram_env/bin/python3 -m pip install --quiet --upgrade pip
.cram_env/bin/pip install "prysk==0.15.0"
