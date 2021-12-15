package update

import (
	"encoding/json"
	"io/ioutil"
	"log"
	"net/http"
	"net/http/httptest"
	"os"
	"testing"
	"time"

	"github.com/stretchr/testify/assert"
)

func TestCheckforUpdate(t *testing.T) {

	tests := []struct {
		name           string
		currentVersion string
		latestVersion  *ReleaseInfo
		update         bool
		lastCheck      time.Time
	}{
		{
			name:           "latest version is the newest",
			currentVersion: "v1.0.6",
			latestVersion: &ReleaseInfo{
				VERSION: "v1.0.9",
			},
		},
		// TODO: add more test cases
	}
	for _, tt := range tests {

		//setup test
		url := setupTestServer(tt.latestVersion)

		// run the tests
		releaseInfo, err := CheckforUpdate(tt.currentVersion, tempFilePath(), url)

		if releaseInfo == nil || err != nil {
			t.Fatal("Expected latest release informations")
		}

		assert.True(t, versionGreaterThan(releaseInfo.VERSION, tt.currentVersion))

	}
}

func setupTestServer(response *ReleaseInfo) string {
	server := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		w.WriteHeader(200)

		_ = json.NewEncoder(w).Encode(response)
	}))

	return server.URL
}
func tempFilePath() string {
	file, err := ioutil.TempFile("", "")
	if err != nil {
		log.Fatal(err)
	}

	os.Remove(file.Name())
	return file.Name()
}
