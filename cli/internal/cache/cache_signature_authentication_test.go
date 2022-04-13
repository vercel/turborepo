// Adapted from ghttps://github.com/thought-machine/please
// Copyright Thought Machine, Inc. or its affiliates. All Rights Reserved.
// SPDX-License-Identifier: Apache-2.0
package cache

import (
	"crypto/hmac"
	"crypto/sha256"
	"encoding/base64"
	"encoding/json"
	"testing"

	"github.com/stretchr/testify/assert"
)

func Test_SecretKeySuccess(t *testing.T) {
	teamId := "team_someid"
	secretKeyEnvName := "TURBO_REMOTE_CACHE_SIGNATURE_KEY"
	secretKeyEnvValue := "my-secret-key-env"
	t.Setenv(secretKeyEnvName, secretKeyEnvValue)

	cases := []struct {
		name                   string
		asa                    *ArtifactSignatureAuthentication
		expectedSecretKey      string
		expectedSecretKeyError bool
	}{
		{
			name: "Accepts secret key",
			asa: &ArtifactSignatureAuthentication{
				teamId:  teamId,
				enabled: true,
			},
			expectedSecretKey:      secretKeyEnvValue,
			expectedSecretKeyError: false,
		},
	}

	for _, tc := range cases {
		t.Run(tc.name, func(t *testing.T) {
			secretKey, err := tc.asa.secretKey()
			if tc.expectedSecretKeyError {
				assert.Error(t, err)
			} else {
				assert.NoError(t, err)
				assert.Equal(t, tc.expectedSecretKey, string(secretKey))
			}
		})
	}
}

func Test_SecretKeyErrors(t *testing.T) {
	teamId := "team_someid"

	// Env secret key TURBO_REMOTE_CACHE_SIGNATURE_KEY is not set

	cases := []struct {
		name                   string
		asa                    *ArtifactSignatureAuthentication
		expectedSecretKey      string
		expectedSecretKeyError bool
	}{
		{
			name: "Secret key not defined errors",
			asa: &ArtifactSignatureAuthentication{
				teamId:  teamId,
				enabled: true,
			},
			expectedSecretKey:      "",
			expectedSecretKeyError: true,
		},
		{
			name: "Secret key is empty errors",
			asa: &ArtifactSignatureAuthentication{
				teamId:  teamId,
				enabled: true,
			},
			expectedSecretKey:      "",
			expectedSecretKeyError: true,
		},
	}

	for _, tc := range cases {
		t.Run(tc.name, func(t *testing.T) {
			secretKey, err := tc.asa.secretKey()
			if tc.expectedSecretKeyError {
				assert.Error(t, err)
			} else {
				assert.NoError(t, err)
				assert.Equal(t, tc.expectedSecretKey, string(secretKey))
			}
		})
	}
}

func Test_GenerateTagAndValidate(t *testing.T) {
	teamId := "team_someid"
	hash := "the-artifact-hash"
	artifactBody := []byte("the artifact body as bytes")
	secretKeyEnvName := "TURBO_REMOTE_CACHE_SIGNATURE_KEY"
	secretKeyEnvValue := "my-secret-key-env"
	t.Setenv(secretKeyEnvName, secretKeyEnvValue)

	cases := []struct {
		name                    string
		asa                     *ArtifactSignatureAuthentication
		expectedTagMatches      string
		expectedTagDoesNotMatch string
	}{
		{
			name: "Uses hash to generate tag",
			asa: &ArtifactSignatureAuthentication{
				teamId:  teamId,
				enabled: true,
			},
			expectedTagMatches:      testUtilGetHMACTag(hash, teamId, artifactBody, secretKeyEnvValue),
			expectedTagDoesNotMatch: testUtilGetHMACTag("wrong-hash", teamId, artifactBody, secretKeyEnvValue),
		},
		{
			name: "Uses teamId to generate tag",
			asa: &ArtifactSignatureAuthentication{
				teamId:  teamId,
				enabled: true,
			},
			expectedTagMatches:      testUtilGetHMACTag(hash, teamId, artifactBody, secretKeyEnvValue),
			expectedTagDoesNotMatch: testUtilGetHMACTag(hash, "wrong-teamId", artifactBody, secretKeyEnvValue),
		},
		{
			name: "Uses artifactBody to generate tag",
			asa: &ArtifactSignatureAuthentication{
				teamId:  teamId,
				enabled: true,
			},
			expectedTagMatches:      testUtilGetHMACTag(hash, teamId, artifactBody, secretKeyEnvValue),
			expectedTagDoesNotMatch: testUtilGetHMACTag(hash, teamId, []byte("wrong-artifact-body"), secretKeyEnvValue),
		},
		{
			name: "Uses secret to generate tag",
			asa: &ArtifactSignatureAuthentication{
				teamId:  teamId,
				enabled: true,
			},
			expectedTagMatches:      testUtilGetHMACTag(hash, teamId, artifactBody, secretKeyEnvValue),
			expectedTagDoesNotMatch: testUtilGetHMACTag(hash, teamId, artifactBody, "wrong-secret"),
		},
	}

	for _, tc := range cases {
		t.Run(tc.name, func(t *testing.T) {
			tag, err := tc.asa.generateTag(hash, artifactBody)
			assert.NoError(t, err)

			// validates the tag
			assert.Equal(t, tc.expectedTagMatches, tag)
			isValid, err := tc.asa.validate(hash, artifactBody, tc.expectedTagMatches)
			assert.NoError(t, err)
			assert.True(t, isValid)

			// does not validate the tag
			assert.NotEqual(t, tc.expectedTagDoesNotMatch, tag)
			isValid, err = tc.asa.validate(hash, artifactBody, tc.expectedTagDoesNotMatch)
			assert.NoError(t, err)
			assert.False(t, isValid)

		})
	}
}

// Test utils

// Return the Base64 encoded HMAC given the artifact metadata and artifact body
func testUtilGetHMACTag(hash string, teamId string, artifactBody []byte, secret string) string {
	artifactMetadata := &struct {
		Hash   string `json:"hash"`
		TeamId string `json:"teamId"`
	}{
		Hash:   hash,
		TeamId: teamId,
	}
	metadata, _ := json.Marshal(artifactMetadata)
	h := hmac.New(sha256.New, []byte(secret))
	h.Write(metadata)
	h.Write(artifactBody)
	return base64.StdEncoding.EncodeToString(h.Sum(nil))
}

func Test_Utils(t *testing.T) {
	teamId := "team_someid"
	secret := "my-secret"
	hash := "the-artifact-hash"
	artifactBody := []byte("the artifact body as bytes")
	testTag := testUtilGetHMACTag(hash, teamId, artifactBody, secret)
	expectedTag := "9Fu8YniPZ2dEBolTPQoNlFWG0LNMW8EXrBsRmf/fEHk="
	assert.True(t, hmac.Equal([]byte(testTag), []byte(expectedTag)))
}
