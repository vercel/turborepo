package cache

import (
	"crypto/hmac"
	"crypto/sha256"
	"encoding/base64"
	"encoding/json"
	"testing"

	"github.com/stretchr/testify/assert"
	"github.com/vercel/turborepo/cli/internal/fs"
)

func Test_SecretKey(t *testing.T) {
	teamId := "team_someid"
	secret := "my-secret"
	secretKeyEnvName := "TURBO_TEST_SIGNING_KEY"
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
				teamId: teamId,
				options: &fs.SignatureOptions{
					Enabled: true,
					Key:     secret,
				},
			},
			expectedSecretKey:      secret,
			expectedSecretKeyError: false,
		},
		{
			name: "Accepts secret keyEnv",
			asa: &ArtifactSignatureAuthentication{
				teamId: teamId,
				options: &fs.SignatureOptions{
					Enabled: true,
					KeyEnv:  secretKeyEnvName,
				},
			},
			expectedSecretKey:      secretKeyEnvValue,
			expectedSecretKeyError: false,
		},
		{
			name: "Prefers secret keyEnv",
			asa: &ArtifactSignatureAuthentication{
				teamId: teamId,
				options: &fs.SignatureOptions{
					Enabled: true,
					Key:     secret,
					KeyEnv:  secretKeyEnvName,
				},
			},
			expectedSecretKey:      secretKeyEnvValue,
			expectedSecretKeyError: false,
		},
		{
			name: "Secret key not defined errors",
			asa: &ArtifactSignatureAuthentication{
				teamId: teamId,
				options: &fs.SignatureOptions{
					Enabled: true,
				},
			},
			expectedSecretKey:      "",
			expectedSecretKeyError: true,
		},
		{
			name: "Secret key is empty errors",
			asa: &ArtifactSignatureAuthentication{
				teamId: teamId,
				options: &fs.SignatureOptions{
					Enabled: true,
					Key:     "",
				},
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
	secret := "my-secret"

	cases := []struct {
		name                    string
		asa                     *ArtifactSignatureAuthentication
		expectedTagMatches      string
		expectedTagDoesNotMatch string
	}{
		{
			name: "Uses hash to generate tag",
			asa: &ArtifactSignatureAuthentication{
				teamId: teamId,
				options: &fs.SignatureOptions{
					Enabled: true,
					Key:     secret,
				},
			},
			expectedTagMatches:      testUtilGetHMACTag(hash, teamId, artifactBody, secret),
			expectedTagDoesNotMatch: testUtilGetHMACTag("wrong-hash", teamId, artifactBody, secret),
		},
		{
			name: "Uses teamId to generate tag",
			asa: &ArtifactSignatureAuthentication{
				teamId: teamId,
				options: &fs.SignatureOptions{
					Enabled: true,
					Key:     secret,
				},
			},
			expectedTagMatches:      testUtilGetHMACTag(hash, teamId, artifactBody, secret),
			expectedTagDoesNotMatch: testUtilGetHMACTag(hash, "wrong-teamId", artifactBody, secret),
		},
		{
			name: "Uses artifactBody to generate tag",
			asa: &ArtifactSignatureAuthentication{
				teamId: teamId,
				options: &fs.SignatureOptions{
					Enabled: true,
					Key:     secret,
				},
			},
			expectedTagMatches:      testUtilGetHMACTag(hash, teamId, artifactBody, secret),
			expectedTagDoesNotMatch: testUtilGetHMACTag(hash, teamId, []byte("wrong-artifact-body"), secret),
		},
		{
			name: "Uses secret to generate tag",
			asa: &ArtifactSignatureAuthentication{
				teamId: teamId,
				options: &fs.SignatureOptions{
					Enabled: true,
					Key:     secret,
				},
			},
			expectedTagMatches:      testUtilGetHMACTag(hash, teamId, artifactBody, secret),
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
