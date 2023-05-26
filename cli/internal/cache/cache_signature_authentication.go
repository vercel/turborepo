// Adapted from https://github.com/thought-machine/please
// Copyright Thought Machine, Inc. or its affiliates. All Rights Reserved.
// SPDX-License-Identifier: Apache-2.0
package cache

import (
	"crypto/hmac"
	"crypto/sha256"
	"encoding/base64"
	"errors"
	"fmt"
	"hash"
	"os"
)

type ArtifactSignatureAuthentication struct {
	teamID string
	// Used for testing purposes
	secretKeyOverride []byte
	enabled           bool
}

func (asa *ArtifactSignatureAuthentication) isEnabled() bool {
	return asa.enabled
}

// If the secret key is not found or the secret key length is 0, an error is returned
// Preference is given to the environment specified secret key.
func (asa *ArtifactSignatureAuthentication) getSecretKey() ([]byte, error) {
	var secret []byte
	if asa.secretKeyOverride != nil {
		secret = asa.secretKeyOverride
	} else {
		secret = []byte(os.Getenv("TURBO_REMOTE_CACHE_SIGNATURE_KEY"))
	}

	if len(secret) == 0 {
		return nil, errors.New("signature secret key not found. You must specify a secret key in the TURBO_REMOTE_CACHE_SIGNATURE_KEY environment variable")
	}
	return secret, nil
}

func (asa *ArtifactSignatureAuthentication) generateTag(hash string, artifactBody []byte) (string, error) {
	tag, err := asa.getTagGenerator(hash)
	if err != nil {
		return "", err
	}
	tag.Write(artifactBody)
	return base64.StdEncoding.EncodeToString(tag.Sum(nil)), nil
}

func (asa *ArtifactSignatureAuthentication) getTagGenerator(hash string) (hash.Hash, error) {
	teamID := asa.teamID
	secret, err := asa.getSecretKey()
	if err != nil {
		return nil, err
	}
	metadata := []byte(hash)
	metadata = append(metadata, []byte(teamID)...)

	// TODO(Gaspar) Support additional signing algorithms here
	h := hmac.New(sha256.New, secret)
	h.Write(metadata)
	return h, nil
}

func (asa *ArtifactSignatureAuthentication) validate(hash string, artifactBody []byte, expectedTag string) (bool, error) {
	computedTag, err := asa.generateTag(hash, artifactBody)
	if err != nil {
		return false, fmt.Errorf("failed to verify artifact tag: %w", err)
	}
	return hmac.Equal([]byte(computedTag), []byte(expectedTag)), nil
}

type StreamValidator struct {
	currentHash hash.Hash
}

func (sv *StreamValidator) Validate(expectedTag string) bool {
	computedTag := base64.StdEncoding.EncodeToString(sv.currentHash.Sum(nil))
	return hmac.Equal([]byte(computedTag), []byte(expectedTag))
}

func (sv *StreamValidator) CurrentValue() string {
	return base64.StdEncoding.EncodeToString(sv.currentHash.Sum(nil))
}
