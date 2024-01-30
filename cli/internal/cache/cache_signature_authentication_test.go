// Adapted from https://github.com/thought-machine/please
// Copyright Thought Machine, Inc. or its affiliates. All Rights Reserved.
// SPDX-License-Identifier: Apache-2.0
package cache

import (
	"crypto/hmac"
	"crypto/sha256"
	"encoding/base64"
	"encoding/hex"
	"fmt"
	"math/rand"
	"testing"

	"github.com/stretchr/testify/assert"
	"github.com/vercel/turbo/cli/internal/edgecases"
	"github.com/vercel/turbo/cli/internal/ffi"
	"github.com/vercel/turbo/cli/internal/xxhash"
)

func Test_SecretKeySuccess(t *testing.T) {
	teamID := "team_someid"
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
				teamID:  teamID,
				enabled: true,
			},
			expectedSecretKey:      secretKeyEnvValue,
			expectedSecretKeyError: false,
		},
	}

	for _, tc := range cases {
		t.Run(tc.name, func(t *testing.T) {
			secretKey, err := tc.asa.getSecretKey()
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
	teamID := "team_someid"

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
				teamID:  teamID,
				enabled: true,
			},
			expectedSecretKey:      "",
			expectedSecretKeyError: true,
		},
		{
			name: "Secret key is empty errors",
			asa: &ArtifactSignatureAuthentication{
				teamID:  teamID,
				enabled: true,
			},
			expectedSecretKey:      "",
			expectedSecretKeyError: true,
		},
	}

	for _, tc := range cases {
		t.Run(tc.name, func(t *testing.T) {
			secretKey, err := tc.asa.getSecretKey()
			if tc.expectedSecretKeyError {
				assert.Error(t, err)
			} else {
				assert.NoError(t, err)
				assert.Equal(t, tc.expectedSecretKey, string(secretKey))
			}
		})
	}
}

var MinimumLength = 10

func generateRandomBytes() []byte {
	length := MinimumLength + rand.Intn(250)
	b := make([]byte, length)
	rand.Read(b)
	return b
}

func generateRandomHash() (string, error) {
	bytes := generateRandomBytes()
	hash := xxhash.New()

	_, err := hash.Write(bytes)

	return hex.EncodeToString(hash.Sum(nil)), err
}

func getRandomEdgecase() string {
	return edgecases.Strings[rand.Intn(len(edgecases.Strings))]
}

func Test_EdgecaseStrings(t *testing.T) {
	TestCases := 1000
	for i := 0; i < TestCases; i++ {
		teamID := getRandomEdgecase()
		hash := getRandomEdgecase()
		artifactBody := getRandomEdgecase()
		secretKey := getRandomEdgecase()
		asa := &ArtifactSignatureAuthentication{
			teamID:            teamID,
			secretKeyOverride: []byte(secretKey),
		}

		tag, err := asa.generateTag(hash, []byte(artifactBody))
		assert.NoError(t, err)

		isValid, err := ffi.VerifySignature([]byte(teamID), hash, []byte(artifactBody), tag, []byte(secretKey))
		assert.NoError(t, err)
		assert.True(t, isValid)
	}
}

func Test_RandomlyGenerateCases(t *testing.T) {
	TestCases := 1000

	for i := 0; i < TestCases; i++ {
		t.Run(fmt.Sprintf("Case %v", i), func(t *testing.T) {
			teamID := generateRandomBytes()
			hash, err := generateRandomHash()
			assert.NoError(t, err)
			artifactBody := generateRandomBytes()
			secretKey := generateRandomBytes()

			asa := &ArtifactSignatureAuthentication{
				teamID:            string(teamID),
				secretKeyOverride: secretKey,
			}

			tag, err := asa.generateTag(hash, artifactBody)
			assert.NoError(t, err)

			isValid, err := ffi.VerifySignature(teamID, hash, artifactBody, tag, secretKey)
			assert.NoError(t, err)
			assert.True(t, isValid)
		})
	}
}

func Test_GenerateTagAndValidate(t *testing.T) {
	teamID := "team_someid"
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
				teamID:  teamID,
				enabled: true,
			},
			expectedTagMatches:      testUtilGetHMACTag(hash, teamID, artifactBody, secretKeyEnvValue),
			expectedTagDoesNotMatch: testUtilGetHMACTag("wrong-hash", teamID, artifactBody, secretKeyEnvValue),
		},
		{
			name: "Uses teamID to generate tag",
			asa: &ArtifactSignatureAuthentication{
				teamID:  teamID,
				enabled: true,
			},
			expectedTagMatches:      testUtilGetHMACTag(hash, teamID, artifactBody, secretKeyEnvValue),
			expectedTagDoesNotMatch: testUtilGetHMACTag(hash, "wrong-teamID", artifactBody, secretKeyEnvValue),
		},
		{
			name: "Uses artifactBody to generate tag",
			asa: &ArtifactSignatureAuthentication{
				teamID:  teamID,
				enabled: true,
			},
			expectedTagMatches:      testUtilGetHMACTag(hash, teamID, artifactBody, secretKeyEnvValue),
			expectedTagDoesNotMatch: testUtilGetHMACTag(hash, teamID, []byte("wrong-artifact-body"), secretKeyEnvValue),
		},
		{
			name: "Uses secret to generate tag",
			asa: &ArtifactSignatureAuthentication{
				teamID:  teamID,
				enabled: true,
			},
			expectedTagMatches:      testUtilGetHMACTag(hash, teamID, artifactBody, secretKeyEnvValue),
			expectedTagDoesNotMatch: testUtilGetHMACTag(hash, teamID, artifactBody, "wrong-secret"),
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

			isValid, err = ffi.VerifySignature([]byte(teamID), hash, artifactBody, tag, nil)
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
func testUtilGetHMACTag(hash string, teamID string, artifactBody []byte, secret string) string {
	metadata := []byte(hash)
	metadata = append(metadata, []byte(teamID)...)
	h := hmac.New(sha256.New, []byte(secret))
	h.Write(metadata)
	h.Write(artifactBody)
	return base64.StdEncoding.EncodeToString(h.Sum(nil))
}

func Test_Utils(t *testing.T) {
	teamID := "team_someid"
	secret := "my-secret"
	hash := "the-artifact-hash"
	artifactBody := []byte("the artifact body as bytes")
	testTag := testUtilGetHMACTag(hash, teamID, artifactBody, secret)
	fmt.Println(testTag)
	expectedTag := "mh3PI05JSXRfAy3hL0Dz3Gjq0UhZYKalu1HwmLNvYjs="
	assert.True(t, hmac.Equal([]byte(testTag), []byte(expectedTag)))
}
