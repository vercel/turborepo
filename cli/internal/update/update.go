// Package update is checking for a new version of Turborepo and informs the user
// to update. Most of the logic is copied from planetscale/cli:
// https://github.com/planetscale/cli/blob/main/internal/update/update.go
// and updated to our own needs.
package update

import (
	"context"
	"encoding/json"
	"fmt"
	"io/ioutil"
	"net/http"
	"os"
	"path/filepath"
	"time"
	"turbo/internal/config"
	"turbo/internal/util"

	"github.com/hashicorp/go-version"
	"gopkg.in/yaml.v2"
)

type UpdateInfo struct {
	Update      bool
	Reason      string
	ReleaseInfo *ReleaseInfo
}

// ReleaseInfo stores information about a release
type ReleaseInfo struct {
	Version     string    `json:"tag_name"`
	URL         string    `json:"html_url"`
	PublishedAt time.Time `json:"published_at"`
}

// StateEntry stores the information we have checked for a new version. It's
// used to decide whether to check for a new version or not.
type StateEntry struct {
	CheckedForUpdateAt time.Time   `yaml:"checked_for_update_at"`
	LatestRelease      ReleaseInfo `yaml:"latest_release"`
}

// CheckVersion checks for the given build version whether there is a new
// version of the CLI or not.
func CheckVersion(ctx context.Context, buildVersion string) error {
	if ctx.Err() != nil {
		return ctx.Err()
	}

	path, err := stateFilePath()
	if err != nil {
		return err
	}

	updateInfo, err := checkVersion(
		ctx,
		buildVersion,
		path,
		latestVersion,
	)
	if err != nil {
		return fmt.Errorf("skipping update, error: %s", err)
	}

	if !updateInfo.Update {
		return fmt.Errorf("skipping update, reason: %s", updateInfo.Reason)
	}

	util.Printf("\n${BLUE}A new release of turborepo is available: ${CYAN}%s → %s\n", buildVersion, updateInfo.ReleaseInfo.Version)
	util.Printf("${YELLOW}%s${RESET}\n", updateInfo.ReleaseInfo.URL)
	return nil
}

func checkVersion(
	ctx context.Context,
	buildVersion, path string,
	latestVersionFn func(ctx context.Context, addr string) (*ReleaseInfo, error),
) (*UpdateInfo, error) {
	if _, exists := os.LookupEnv("TURBO_NO_UPDATE_NOTIFIER"); exists {
		return &UpdateInfo{
			Update: false,
			Reason: "TURBO_NO_UPDATE_NOTIFIER is set",
		}, nil
	}

	stateEntry, _ := getStateEntry(path)
	if stateEntry != nil && time.Since(stateEntry.CheckedForUpdateAt).Hours() < 24 {
		return &UpdateInfo{
			Update: false,
			Reason: "Latest version was already checked",
		}, nil
	}

	addr := "https://api.github.com/repos/vercel/turborepo/releases/latest"
	info, err := latestVersionFn(ctx, addr)
	if err != nil {
		return nil, err
	}

	err = setStateEntry(path, time.Now(), *info)
	if err != nil {
		return nil, err
	}

	v1, err := version.NewVersion(info.Version)
	if err != nil {
		return nil, err
	}

	v2, err := version.NewVersion(buildVersion)
	if err != nil {
		return nil, err
	}

	if v1.LessThanOrEqual(v2) {
		return &UpdateInfo{
			Update: false,
			Reason: fmt.Sprintf("Latest version (%s) is less than or equal to current build version (%s)",
				info.Version, buildVersion),
			ReleaseInfo: info,
		}, nil
	}

	return &UpdateInfo{
		Update: true,
		Reason: fmt.Sprintf("Latest version (%s) is greater than the current build version (%s)",
			info.Version, buildVersion),
		ReleaseInfo: info,
	}, nil

}

func latestVersion(ctx context.Context, addr string) (*ReleaseInfo, error) {
	req, err := http.NewRequestWithContext(ctx, "GET", addr, nil)
	if err != nil {
		return nil, err
	}

	req.Header.Set("Content-Type", "application/json; charset=utf-8")
	req.Header.Set("Accept", "application/vnd.github.v3+json")

	getToken := func() string {
		if t := os.Getenv("GH_TOKEN"); t != "" {
			return t
		}
		return os.Getenv("GITHUB_TOKEN")
	}

	if token := getToken(); token != "" {
		req.Header.Set("Authorization", fmt.Sprintf("token %s", token))
	}

	client := &http.Client{Timeout: time.Second * 15}
	resp, err := client.Do(req)
	if err != nil {
		return nil, err
	}
	defer resp.Body.Close()

	out, err := ioutil.ReadAll(resp.Body)
	if err != nil {
		return nil, err
	}

	success := resp.StatusCode >= 200 && resp.StatusCode < 300
	if !success {
		return nil, fmt.Errorf("error fetching latest release: %v", string(out))
	}

	var info *ReleaseInfo
	err = json.Unmarshal(out, &info)
	if err != nil {
		return nil, err
	}

	return info, nil
}

func getStateEntry(stateFilePath string) (*StateEntry, error) {
	content, err := ioutil.ReadFile(stateFilePath)
	if err != nil {
		return nil, err
	}

	var stateEntry StateEntry
	err = yaml.Unmarshal(content, &stateEntry)
	if err != nil {
		return nil, err
	}

	return &stateEntry, nil
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
	_ = ioutil.WriteFile(stateFilePath, content, 0600)

	return nil
}

func stateFilePath() (string, error) {
	dir, err := config.GetConfigDir()
	if err != nil {
		return "", err
	}

	return filepath.Join(dir, "state.yml"), nil
}
