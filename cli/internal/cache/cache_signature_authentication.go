package cache

import (
	"crypto/hmac"
	"crypto/sha256"
	"encoding/base64"
	"encoding/json"
	"errors"
	"fmt"
	"os"

	"github.com/vercel/turborepo/cli/internal/fs"
)

type ArtifactSignatureAuthentication struct {
	teamId  string
	options *fs.SignatureOptions
}

func (sv *ArtifactSignatureAuthentication) isEnabled() bool {
	return sv.options.Enabled
}

func (sv *ArtifactSignatureAuthentication) generateTag(hash string, artifactBody []byte) (string, error) {
	teamId := sv.teamId
	secret, err := sv.secretKey()
	if err != nil {
		return "", err
	}
	artifactMetadata := &struct {
		Hash   string `json:"hash"`
		TeamId string `json:"teamId"`
	}{
		Hash:   hash,
		TeamId: teamId,
	}
	metadata, err := json.Marshal(artifactMetadata)
	if err != nil {
		return "", err
	}

	h := hmac.New(sha256.New, secret)
	h.Write(metadata)
	h.Write(artifactBody)
	tag := base64.StdEncoding.EncodeToString(h.Sum(nil))
	return tag, nil
}

func (sv *ArtifactSignatureAuthentication) validateTag(hash string, artifactBody []byte, expectedTag string) (bool, error) {
	computedTag, err := sv.generateTag(hash, artifactBody)
	if err != nil {
		return false, fmt.Errorf("failed to verify artifact tag: %w", err)
	}
	return hmac.Equal([]byte(computedTag), []byte(expectedTag)), nil
}

func (sv *ArtifactSignatureAuthentication) secretKey() ([]byte, error) {
	secret := ""
	switch {
	case len(sv.options.Key) > 0:
		secret = sv.options.Key
	case len(sv.options.KeyEnv) > 0:
		secret = os.Getenv(sv.options.KeyEnv)
	}
	if len(secret) == 0 {
		return nil, errors.New("signature secret key not found. You must specify a secret key or keyEnv name in your turbo.json config")
	}
	return []byte(secret), nil
}
