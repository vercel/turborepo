#!/usr/bin/env bash

set -e

if [[ -z "$GPG_KEY" ]]; then
	GPG_KEY=/secrets/key.gpg
fi

if [[ -f "${GPG_KEY}" ]]; then
	echo "importing gpg key..."
	if gpg --batch --import "${GPG_KEY}"; then
		gpg --list-secret-keys --keyid-format long
	fi
fi

if [[ -z "$DOCKER_CREDS_FILE" ]]; then
	DOCKER_CREDS_FILE=/secrets/.docker-creds
fi

function docker-login() {
	if ! echo "$2" | docker login "$3" --username "$1" --password-stdin ; then
		if [[ $DOCKER_FAIL_ON_LOGIN_ERROR == "true" ]]; then
			exit 1
		fi
	fi
}

if [[ -f $DOCKER_CREDS_FILE ]]; then
	IFS=':'
	while read -r user password registry; do
		echo "$user" "$password" "$registry"
		docker-login "$user" "$password" "$registry"
	done <$DOCKER_CREDS_FILE
else
	if [[ -n "${DOCKER_USERNAME}" ]]; then
		docker-login "${DOCKER_USERNAME}" "${DOCKER_PASSWORD}" "$DOCKER_HOST"
	fi
fi

exec goreleaser "$@"
