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

func Test_SignatureAuthentication(t *testing.T) {
	teamId := "team_someid"
	secret := "my-secret"
	hash := "the-artifact-hash"
	artifactBody := []byte("the artifact body as bytes")
	signerVerifier := &ArtifactSignatureAuthentication{
		teamId: teamId,
		options: &fs.SignatureOptions{
			Enabled: true,
			Key:     secret,
		},
	}
	expectedTag := testUtilGetHMACTag(hash, teamId, artifactBody, secret)

	// Test methods
	assert.EqualValues(t, true, signerVerifier.isEnabled())
	testKey, err := signerVerifier.secretKey()
	if err != nil {
		t.Fatalf("Error retrieving key %#v", err)
	}
	assert.EqualValues(t, []byte(secret), testKey)
	tag, err := signerVerifier.generateTag(hash, artifactBody)
	if err != nil {
		t.Fatalf("Error generating tag: %#v", err)
	}
	assert.EqualValues(t, expectedTag, tag)
	signatureIsValid, err := signerVerifier.validateTag(hash, artifactBody, expectedTag)
	if err != nil {
		t.Fatalf("Error generating tag: %#v", err)
	}
	assert.True(t, signatureIsValid)
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
