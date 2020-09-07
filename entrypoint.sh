#!/usr/bin/env bash

if [[ -z "${PRIVATE_KEY}" ]]; then
    echo "can not find any private key, ignore..."
else
    key_file=$HOME/key.asc
    echo -e "${PRIVATE_KEY}" | base64 -d >"$key_file"
    echo "save key to $key_file"

    if gpg --allow-secret-key-import --import "${key_file}"; then
        gpg --list-secret-keys --keyid-format long
    fi
    rm -rf "$key_file"
fi

goreleaser "$@"
