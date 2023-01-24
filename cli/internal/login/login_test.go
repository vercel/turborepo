package login

import (
	"context"
	"fmt"
	"net/http"
	"net/url"
	"os"
	"testing"

	"github.com/hashicorp/go-hclog"
	"github.com/pkg/errors"
	"github.com/spf13/pflag"
	"github.com/vercel/turbo/cli/internal/client"
	"github.com/vercel/turbo/cli/internal/cmdutil"
	"github.com/vercel/turbo/cli/internal/config"
	"github.com/vercel/turbo/cli/internal/fs"
	"github.com/vercel/turbo/cli/internal/turbopath"
	"github.com/vercel/turbo/cli/internal/ui"
	"github.com/vercel/turbo/cli/internal/util"
	"gotest.tools/v3/assert"
)

type dummyClient struct {
	setToken            string
	createdSSOTokenName string
	team                *client.Team
	cachingStatus       util.CachingStatus
}

func (d *dummyClient) SetToken(t string) {
	d.setToken = t
}

func (d *dummyClient) GetUser() (*client.UserResponse, error) {
	return &client.UserResponse{}, nil
}

func (d *dummyClient) GetTeam(teamID string) (*client.Team, error) {
	return d.team, nil
}

func (d *dummyClient) GetCachingStatus() (util.CachingStatus, error) {
	return d.cachingStatus, nil
}

func (d *dummyClient) SetTeamID(teamID string) {}

func (d *dummyClient) VerifySSOToken(token string, tokenName string) (*client.VerifiedSSOUser, error) {
	d.createdSSOTokenName = tokenName
	return &client.VerifiedSSOUser{
		Token:  "actual-sso-token",
		TeamID: "sso-team-id",
	}, nil
}

type testResult struct {
	repoRoot            turbopath.AbsoluteSystemPath
	userConfig          *config.UserConfig
	repoConfig          *config.RepoConfig
	clientErr           error
	openedURL           string
	stepCh              chan struct{}
	client              dummyClient
	shouldEnableCaching bool
}

func (tr *testResult) getTestLogin() login {
	urlOpener := func(url string) error {
		tr.openedURL = url
		tr.stepCh <- struct{}{}
		return nil
	}
	base := &cmdutil.CmdBase{
		UI:         ui.Default(),
		Logger:     hclog.Default(),
		RepoRoot:   tr.repoRoot,
		UserConfig: tr.userConfig,
		RepoConfig: tr.repoConfig,
	}
	return login{
		base:    base,
		openURL: urlOpener,
		client:  &tr.client,
		promptEnableCaching: func() (bool, error) {
			return tr.shouldEnableCaching, nil
		},
	}
}

func newTest(t *testing.T, redirectedURL string) *testResult {
	t.Helper()
	stepCh := make(chan struct{}, 1)
	flags := pflag.NewFlagSet("test-flags", pflag.ContinueOnError)
	config.AddUserConfigFlags(flags)
	config.AddRepoConfigFlags(flags)
	assert.NilError(t, flags.Set("login", "login-url"))
	assert.NilError(t, flags.Set("api", "api-url"))
	userConfigPath := fs.AbsoluteSystemPathFromUpstream(t.TempDir()).UntypedJoin("turborepo")
	userConfig, err := config.ReadUserConfigFile(userConfigPath, config.FlagSet{FlagSet: flags})
	if err != nil {
		t.Fatalf("setting up user config: %v", err)
	}
	repoRoot := fs.AbsoluteSystemPathFromUpstream(t.TempDir())
	repoConfig, err := config.ReadRepoConfigFile(config.GetRepoConfigPath(repoRoot), config.FlagSet{FlagSet: flags})
	if err != nil {
		t.Fatalf("setting up repo config: %v", err)
	}
	tr := &testResult{
		repoRoot:   repoRoot,
		userConfig: userConfig,
		repoConfig: repoConfig,
		stepCh:     stepCh,
	}
	tr.client.team = &client.Team{
		ID:         "sso-team-id",
		Membership: client.Membership{Role: "OWNER"},
	}
	tr.client.cachingStatus = util.CachingStatusEnabled
	// When it's time, do the redirect
	go func() {
		<-tr.stepCh
		client := &http.Client{
			CheckRedirect: func(req *http.Request, via []*http.Request) error {
				return http.ErrUseLastResponse
			},
		}
		resp, err := client.Get(redirectedURL)
		if err != nil {
			tr.clientErr = err
		} else if resp != nil && resp.StatusCode != http.StatusFound {
			tr.clientErr = fmt.Errorf("invalid status %v", resp.StatusCode)
		}
	}()
	return tr
}

func Test_run(t *testing.T) {
	ctx := context.Background()
	test := newTest(t, "http://127.0.0.1:9789/?token=my-token")
	login := test.getTestLogin()
	err := login.run(ctx)
	if err != nil {
		t.Errorf("expected to succeed, got error %v", err)
	}
	if test.clientErr != nil {
		t.Errorf("test client had error %v", test.clientErr)
	}

	expectedURL := "login-url/turborepo/token?redirect_uri=http://127.0.0.1:9789"
	if test.openedURL != expectedURL {
		t.Errorf("openedURL got %v, want %v", test.openedURL, expectedURL)
	}

	if test.userConfig.Token() != "my-token" {
		t.Errorf("config token got %v, want my-token", test.userConfig.Token())
	}
	if test.client.setToken != "my-token" {
		t.Errorf("user client token got %v, want my-token", test.client.setToken)
	}
}

func Test_sso(t *testing.T) {
	ctx := context.Background()
	redirectParams := make(url.Values)
	redirectParams.Add("token", "verification-token")
	redirectParams.Add("email", "test@example.com")
	test := newTest(t, "http://127.0.0.1:9789/?"+redirectParams.Encode())
	login := test.getTestLogin()
	err := login.loginSSO(ctx, "my-team")
	if err != nil {
		t.Errorf("expected to succeed, got error %v", err)
	}
	if test.clientErr != nil {
		t.Errorf("test client had error %v", test.clientErr)
	}
	host, err := os.Hostname()
	if err != nil {
		t.Errorf("failed to get hostname %v", err)
	}
	expectedTokenName := fmt.Sprintf("Turbo CLI on %v via SAML/OIDC Single Sign-On", host)
	if test.client.createdSSOTokenName != expectedTokenName {
		t.Errorf("created sso token got %v want %v", test.client.createdSSOTokenName, expectedTokenName)
	}
	expectedToken := "actual-sso-token"
	if test.client.setToken != expectedToken {
		t.Errorf("user client token got %v, want %v", test.client.setToken, expectedToken)
	}

	if test.userConfig.Token() != expectedToken {
		t.Errorf("user config token got %v want %v", test.userConfig.Token(), expectedToken)
	}
	remoteConfig := test.repoConfig.GetRemoteConfig(expectedToken)
	expectedTeamID := "sso-team-id"
	if remoteConfig.TeamID != expectedTeamID {
		t.Errorf("repo config team id got %v want %v", remoteConfig.TeamID, expectedTeamID)
	}
}

func Test_ssoCachingDisabledShouldEnable(t *testing.T) {
	ctx := context.Background()
	redirectParams := make(url.Values)
	redirectParams.Add("token", "verification-token")
	redirectParams.Add("email", "test@example.com")
	test := newTest(t, "http://127.0.0.1:9789/?"+redirectParams.Encode())
	test.shouldEnableCaching = true
	test.client.cachingStatus = util.CachingStatusDisabled
	login := test.getTestLogin()
	err := login.loginSSO(ctx, "my-team")
	// Handle URL Open
	<-test.stepCh
	if !errors.Is(err, errTryAfterEnable) {
		t.Errorf("loginSSO got %v, want %v", err, errTryAfterEnable)
	}
}

func Test_ssoCachingDisabledDontEnable(t *testing.T) {
	ctx := context.Background()
	redirectParams := make(url.Values)
	redirectParams.Add("token", "verification-token")
	redirectParams.Add("email", "test@example.com")
	test := newTest(t, "http://127.0.0.1:9789/?"+redirectParams.Encode())
	test.shouldEnableCaching = false
	test.client.cachingStatus = util.CachingStatusDisabled
	login := test.getTestLogin()
	err := login.loginSSO(ctx, "my-team")
	if !errors.Is(err, errNeedCachingEnabled) {
		t.Errorf("loginSSO got %v, want %v", err, errNeedCachingEnabled)
	}
}
