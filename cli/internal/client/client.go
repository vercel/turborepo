package client

import (
	"errors"
	"fmt"
	"io"

	"github.com/hashicorp/go-hclog"
)

type Client interface {
	IsLoggedIn() bool
	SetToken(token string)
	PutArtifact(hash string, artifactBody interface{}, duration int, tag string) error
	FetchArtifact(hash string, rawBody interface{}) (*ClientResponse, error)
	RecordAnalyticsEvents(events []map[string]interface{}) error
}

type ClientType string

const (
	VercelClientType ClientType = "vercel"
	BucketClientType ClientType = "bucket"
)

type ClientConfig struct {
	ClientType         ClientType
	ApiUrl             string
	TeamId             string
	TeamSlug           string
	Token              string
	BucketRegion       string
	BucketName         string
	BucketPrefix       string
	BucketPartition    string
	BucketPathStyle    bool
	AccessKeyId        string
	SecretAccessKey    string
	MaxRemoteFailCount uint64
	TurboVersion       string
	Logger             hclog.Logger
}

func New(config *ClientConfig) (Client, error) {
	switch config.ClientType {
	case VercelClientType:
		return newRemoteClient(config)
	case BucketClientType:
		return newBucketClient(config)
	}
	return nil, fmt.Errorf("invalid ClientType %v", config.ClientType)
}

type ClientResponse struct {
	StatusCode       int
	ArtifactDuration int
	Body             io.ReadCloser
	Tag              string
}

// ErrTooManyFailures is returned from remote cache API methods after `maxRemoteFailCount` errors have occurred
var ErrTooManyFailures = errors.New("skipping HTTP Request, too many failures have occurred")
