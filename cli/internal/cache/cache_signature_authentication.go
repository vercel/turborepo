package cache

import (
	"crypto/hmac"
	"crypto/sha256"
	"encoding/base64"
	"encoding/json"
	"errors"
	"fmt"
	"hash"
	"io"
	"os"

	"github.com/vercel/turborepo/cli/internal/fs"
)

type ArtifactSignatureAuthentication struct {
	teamId  string
	options *fs.SignatureOptions
}

func (asa *ArtifactSignatureAuthentication) isEnabled() bool {
	return asa.options.Enabled
}

// If the secret key is not found or the secret key length is 0, an error is returned
// Preference is given to the enviornment specifed secret key.
func (asa *ArtifactSignatureAuthentication) secretKey() ([]byte, error) {
	secret := ""
	switch {
	case len(asa.options.KeyEnv) > 0:
		secret = os.Getenv(asa.options.KeyEnv)
	case len(asa.options.Key) > 0:
		secret = asa.options.Key
	}
	if len(secret) == 0 {
		return nil, errors.New("signature secret key not found. You must specify a secret key or keyEnv name in your turbo.json config")
	}
	return []byte(secret), nil
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
	teamId := asa.teamId
	secret, err := asa.secretKey()
	if err != nil {
		return nil, err
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
		return nil, err
	}

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

func (asa *ArtifactSignatureAuthentication) streamValidator(hash string, incomingReader io.ReadCloser) (io.ReadCloser, *StreamValidator, error) {
	tag, err := asa.getTagGenerator(hash)
	if err != nil {
		return nil, nil, err
	}

	tee := io.TeeReader(incomingReader, tag)
	artifactReader := readCloser{tee, incomingReader}
	return artifactReader, &StreamValidator{tag}, nil
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

type readCloser struct {
	io.Reader
	io.Closer
}
