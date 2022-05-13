package cache

import (
	"errors"
	"net/http"
	"testing"

	"github.com/vercel/turborepo/cli/internal/util"
)

type errorResp struct {
	err error
}

func (sr *errorResp) PutArtifact(hash string, body []byte, duration int, tag string) error {
	return sr.err
}

func (sr *errorResp) FetchArtifact(hash string) (*http.Response, error) {
	return nil, sr.err
}

func TestRemoteCachingDisabled(t *testing.T) {
	clientErr := &util.CacheDisabledError{
		Status:  util.CachingStatusDisabled,
		Message: "Remote Caching has been disabled for this team. A team owner can enable it here: $URL",
	}
	client := &errorResp{err: clientErr}
	cache := &httpCache{
		client:         client,
		requestLimiter: make(limiter, 20),
	}
	cd := &util.CacheDisabledError{}
	_, _, _, err := cache.Fetch("unused-target", "some-hash", []string{"unused", "outputs"})
	if !errors.As(err, &cd) {
		t.Errorf("cache.Fetch err got %v, want a CacheDisabled error", err)
	}
	if cd.Status != util.CachingStatusDisabled {
		t.Errorf("CacheDisabled.Status got %v, want %v", cd.Status, util.CachingStatusDisabled)
	}
}

// Note that testing Put will require mocking the filesystem and is not currently the most
// interesting test. The current implementation directly returns the error from PutArtifact.
// We should still add the test once feasible to avoid future breakage.
