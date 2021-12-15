package update

import (
	"encoding/json"
	"fmt"
	"io/ioutil"
	"net/http"
	"regexp"
	"strconv"
	"strings"
	"time"

	"github.com/hashicorp/go-version"
	"gopkg.in/yaml.v2"
)

var gitDescribeSuffixRE = regexp.MustCompile(`\d+-\d+-g[a-f0-9]{8}$`)

// ReleaseInfo stores information about a release
type ReleaseInfo struct {
	VERSION     string    `json:"tag_name"`
	URL         string    `json:"html_url"`
	PublishedAt time.Time `json:"published_at"`
}

// StateEntry stores the information we have checked for a new version. It's
// used to decide whether to check for a new version or not.
type StateEntry struct {
	CheckedForUpdateAt time.Time   `yaml:"checked_for_update_at"`
	LatestRelease      ReleaseInfo `yaml:"latest_release"`
}

func CheckforUpdate(currentVersion, stateFilePath, repo string) (*ReleaseInfo, error) {

	// stops looking for an update if it was done in the last 24 hours
	stateEntry, _ := getStateEntry(stateFilePath)
	if stateEntry != nil && time.Since(stateEntry.CheckedForUpdateAt).Hours() < 24 {
		return nil, nil
	}

	releaseInfo, err := getLatestVersion(repo)
	if err != nil {
		return nil, err
	}

	// store the time when the check was done plus release info into a file
	err = setStateEntry(stateFilePath, time.Now(), *releaseInfo)
	if err != nil {
		return nil, err
	}

	if versionGreaterThan(releaseInfo.VERSION, currentVersion) {
		return releaseInfo, nil
	}

	return nil, nil

}

func getLatestVersion(repo string) (*ReleaseInfo, error) {

	req, err := http.NewRequest("GET", repo, nil)
	if err != nil {
		return nil, err
	}
	req.Header.Set("Content-Type", "application/json; charset=utf-8")

	client := &http.Client{Timeout: time.Second * 10}

	response, err := client.Do(req)
	if err != nil {
		return nil, err
	}

	body, err := ioutil.ReadAll(response.Body)
	if err != nil {
		return nil, err
	}

	if !(response.StatusCode >= 200 && response.StatusCode < 300) {
		fmt.Println(response.StatusCode)
		return nil, fmt.Errorf("error fetching latest release: %s", string(body))
	}

	var realeasInfo *ReleaseInfo
	err = json.Unmarshal(body, &realeasInfo)
	if err != nil {
		return nil, err
	}

	return realeasInfo, nil

}

func getStateEntry(stateFilePath string) (*StateEntry, error) {

	content, err := ioutil.ReadFile(stateFilePath)
	if err != nil {
		return nil, err
	}

	var stateEntry *StateEntry
	err = yaml.Unmarshal(content, &stateEntry)

	if err != nil {
		return nil, err
	}

	return stateEntry, nil
}

func setStateEntry(stateFilePath string, t time.Time, r ReleaseInfo) error {
	data := StateEntry{
		CheckedForUpdateAt: t,
		LatestRelease:      r,
	}

	content, err := yaml.Marshal(data)
	if err != nil {
		return err
	}
	err = ioutil.WriteFile(stateFilePath, content, 0600)

	return err
}

func versionGreaterThan(v, w string) bool {
	w = gitDescribeSuffixRE.ReplaceAllStringFunc(w, func(m string) string {
		idx := strings.IndexRune(m, '-')
		n, _ := strconv.Atoi(m[0:idx])
		return fmt.Sprintf("%d-pre.0", n+1)
	})

	vv, ve := version.NewVersion(v)
	vw, we := version.NewVersion(w)

	return ve == nil && we == nil && vv.GreaterThan(vw)
}
