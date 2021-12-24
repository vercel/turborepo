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
	"strings"
	"time"
	"turbo/internal/backends"
	"turbo/internal/config"
	"turbo/internal/util"
	"unicode/utf8"

	cleanhttp "github.com/hashicorp/go-cleanhttp"

	"github.com/hashicorp/go-version"
	"gopkg.in/yaml.v2"
)

const RELEASE_URL = "https://api.github.com/repos/vercel/turborepo/releases/latest"

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
	CheckedForUpdateAt time.Time   `yaml:"checked-for-update-at"`
	LatestRelease      ReleaseInfo `yaml:"latest-release"`
}

// CheckVersion checks for the given build version whether there is a new
// version of the CLI or not.
func CheckVersion(ctx context.Context, config *config.Config, buildVersion string) error {
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
		config.Logger.Debug("No update available", "latest version", updateInfo.ReleaseInfo.Version)
		return nil
	}

	if !updateInfo.Update {
		config.Logger.Debug("No update available, latest version is currently installed", "version", buildVersion)
		return nil
	}

	backend, err := backends.GetBackend()
	if err != nil {
		return fmt.Errorf("cannot infer language backend and package manager: %w", err)
	}
	installCmdStr := strings.Join(backend.GetTurboInstallCommand(), " ")
	installCommandLen := utf8.RuneCountInString((installCmdStr))
	util.Printf("${YELLOW}+----------------------------------------------------------------+${RESET}\n")
	util.Printf("${YELLOW}|${RESET}                                                                ${YELLOW}|${RESET}\n")
	util.Printf("${YELLOW}|${RESET}    Update available for turbo: ${GREY}%s${RESET} → ${CYAN}%s${RESET}%s${YELLOW}|${RESET}\n", fmt.Sprintf("v%s", buildVersion), updateInfo.ReleaseInfo.Version, strings.Repeat(" ", 14-len(updateInfo.ReleaseInfo.Version)))
	util.Printf("${YELLOW}|${RESET}    Run ${CYAN}%s${RESET} to update%s${YELLOW}|${RESET}\n", installCmdStr, strings.Repeat(" ", 46-installCommandLen))
	util.Printf("${YELLOW}|${RESET}                                                                ${YELLOW}|${RESET}\n")
	util.Printf("${YELLOW}+----------------------------------------------------------------+${RESET}\n")
	util.Printf("\n")
	util.Printf("${GREY}For more information and release notes, visit:${RESET}\n")
	util.Printf("${GREY}${UNDERLINE}%s${RESET}\n", updateInfo.ReleaseInfo.URL)
	util.Printf("\n")
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
	if stateEntry != nil && time.Since(stateEntry.CheckedForUpdateAt).Hours() < 1 {
		return &UpdateInfo{
			Update: false,
			Reason: "Latest version was already checked",
		}, nil
	}

	addr := RELEASE_URL
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
	cleanClient := cleanhttp.DefaultClient()
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

	cleanClient.Timeout = time.Second * 15

	resp, err := cleanClient.Do(req)
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
	return config.GetConfigFilePath("state.yml")
}
