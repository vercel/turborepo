package update

import (
	"context"
	"encoding/json"
	"net/http"
	"net/http/httptest"
	"path/filepath"
	"testing"
	"time"

	"github.com/stretchr/testify/assert"
)

func TestLatestVersion(t *testing.T) {

	var tests = []struct {
		name       string
		resp       *ReleaseInfo
		statusCode int
	}{
		{
			name:       "valid response",
			statusCode: 200,
			resp: &ReleaseInfo{
				Version: "v0.1.0",
			},
		},
		{
			name:       "non valid response",
			statusCode: 400,
		},
	}
	for _, tt := range tests {
		tt := tt
		t.Run(tt.name, func(t *testing.T) {
			ts := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
				w.WriteHeader(tt.statusCode)
				_ = json.NewEncoder(w).Encode(tt.resp)
			}))
			defer ts.Close()

			info, err := latestVersion(context.Background(), ts.URL)

			success := tt.statusCode >= 200 && tt.statusCode < 300
			if !success {
				assert.NotNil(t, err)
			} else {
				assert.Nil(t, err)
				assert.EqualValues(t, tt.resp, info)

			}

		})
	}

}

func TestCheckVersion(t *testing.T) {

	var tests = []struct {
		name          string
		buildVersion  string
		latestVersion string
		update        bool
		lastChecked   time.Time
	}{
		{
			name:          "new version",
			buildVersion:  "v0.1.0",
			latestVersion: "v0.2.0",
			update:        true,
		},
		{
			name:          "same version",
			buildVersion:  "v0.2.0",
			latestVersion: "v0.2.0",
			update:        false,
		},
		{
			name:          "higher version",
			buildVersion:  "v0.3.0",
			latestVersion: "v0.2.0",
			update:        false,
		},
		{
			name:          "new version, but we already checked in the past 24 hours",
			buildVersion:  "v0.1.0",
			latestVersion: "v0.2.0",
			update:        false,
			lastChecked:   time.Now().Add(-time.Hour),
		},
	}
	for _, tt := range tests {
		tt := tt
		t.Run(tt.name, func(t *testing.T) {

			dir := t.TempDir()
			path := filepath.Join(dir, "state.yml")

			if !tt.lastChecked.IsZero() {
				err := setStateEntry(path, tt.lastChecked, ReleaseInfo{Version: tt.latestVersion})
				assert.Nil(t, err)
			}

			updateInfo, err := checkVersion(
				context.Background(),
				tt.buildVersion,
				path,
				func(ctx context.Context, addr string) (*ReleaseInfo, error) {
					return &ReleaseInfo{Version: tt.latestVersion}, nil
				},
			)

			assert.Nil(t, err)
			assert.EqualValues(t, tt.update, updateInfo.Update)

		})
	}

}
