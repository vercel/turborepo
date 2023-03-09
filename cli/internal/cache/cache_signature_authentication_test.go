// Adapted from ghttps://github.com/thought-machine/please
// Copyright Thought Machine, Inc. or its affiliates. All Rights Reserved.
// SPDX-License-Identifier: Apache-2.0
package cache

import (
	"crypto/hmac"
	"crypto/sha256"
	"encoding/base64"
	"encoding/json"
	"fmt"
	"math/rand"
	"os"
	"strings"
	"testing"

	"github.com/stretchr/testify/assert"
)

var MinArtifactBodyLength = 100
var MaxArtifactBodyLength = 1000
var MinWordLength = 10
var MaxWordLength = 100
var ValidAsciiChars = 90

var letters = []rune("abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ[]{}()<>!@#$%^&*0123456789~`-_=+|;:',./? ")

func generateWord() string {
	length := MinWordLength + rand.Intn(MaxWordLength-MinWordLength)
	word := make([]rune, length)
	for i := range word {
		randIndex := rand.Intn(len(letters))
		word[i] = letters[randIndex]
	}

	return string(word)
}

func generateTestCase(t *testing.T, r *rand.Rand) string {
	teamId := generateWord()
	hash := generateWord()

	artifactBodyLen := MinArtifactBodyLength + rand.Intn(MaxArtifactBodyLength-MinArtifactBodyLength)
	artifactBody := make([]byte, artifactBodyLen)
	r.Read(artifactBody)

	secretKey := generateWord()

	artifactBodyString := make([]string, artifactBodyLen)
	for i := range artifactBody {
		artifactBodyString[i] = fmt.Sprintf("%d", artifactBody[i])
	}
	t.Setenv("TURBO_REMOTE_CACHE_SIGNATURE_KEY", secretKey)
	hmacTag := testUtilGetHMACTag(hash, teamId, artifactBody, secretKey)

	return fmt.Sprintf("TestCase {") +
		fmt.Sprintf("team_id: \"%v\", secret_key: \"%v\", artifact_hash: \"%v\", ", teamId, secretKey, hash) +
		fmt.Sprintf("artifact_body: vec![%v], hmac_tag: \"%v\" },", strings.Join(artifactBodyString, ", "), hmacTag)
}

func Test_GenerateTestCases(t *testing.T) {
	r := rand.New(rand.NewSource(99))
	testCases := make([]string, 100)

	for i := range testCases {
		testCases[i] = generateTestCase(t, r)
	}
	output := "struct TestCase {\n    team_id: &'static str,\n    secret_key: &'static str,\n    artifact_body: Vec<u8>, artifact_hash: &'static str, \n    hmac_tag: &'static str,\n}\n" +
		fmt.Sprintf("fn get_test_cases() -> Vec<TestCase> { vec![%v] }", strings.Join(testCases, "\n"))

	os.WriteFile("signature_authentication_test_cases.rs", []byte(output), 0644)
}
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
