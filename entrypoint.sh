#!/usr/bin/env bash

if [[ -z "$GPG_KEY" ]]; then
	GPG_KEY=/secrets/key.gpg
fi

if [[ -f "${GPG_KEY}" ]]; then
	echo "importing gpg key..."
	if gpg --allow-secret-key-import --import "${GPG_KEY}"; then
		gpg --list-secret-keys --keyid-format long
	fi
fi

if [[ -z "$DOCKER_CREDS_FILE" ]]; then
	DOCKER_CREDS_FILE=/secrets/.docker-creds
fi

function docker-login() {
	if echo "$2" | docker login "$3" --username "$1" --password-stdin ; then
		echo "SUCCESS: docker login to $registry"
	else
		echo "\033[91mERROR: docker login to $registry\033[0m"
		if [[ $DOCKER_FAIL_ON_LOGIN_ERROR == "true" ]]; then
			exit 1
		fi
	fi
}

if [[ -f $DOCKER_CREDS_FILE ]]; then
	IFS=':'
	while read -r user password registry; do
		if [[ -z "$registry" ]]; then
			registry=hub.docker.io
		fi
		docker-login "$user" "$password" "$registry"
	done <$DOCKER_CREDS_FILE
else
	if [[ -n "${DOCKER_USERNAME}" ]]; then
		if [[ -z "$DOCKER_HOST" ]]; then
			DOCKER_HOST=hub.docker.io
		fi
		docker-login "${DOCKER_USERNAME}" "${DOCKER_PASSWORD}" "$DOCKER_HOST"
	fi
fi

goreleaser "$@"
