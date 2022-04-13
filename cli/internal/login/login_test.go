package login

import (
	"fmt"
	"net/http"
	"net/url"
	"os"
	"testing"

	"github.com/hashicorp/go-hclog"
	"github.com/vercel/turborepo/cli/internal/client"
	"github.com/vercel/turborepo/cli/internal/config"
	"github.com/vercel/turborepo/cli/internal/ui"
)

type dummyClient struct {
	setToken            string
	createdSSOTokenName string
}

func (d *dummyClient) SetToken(t string) {
	d.setToken = t
}

func (d *dummyClient) GetUser() (*client.UserResponse, error) {
	return &client.UserResponse{}, nil
}

func (d *dummyClient) VerifySSOToken(token string, tokenName string) (*client.VerifiedSSOUser, error) {
	d.createdSSOTokenName = tokenName
	return &client.VerifiedSSOUser{
		Token:  "actual-sso-token",
		TeamID: "sso-team-id",
	}, nil
}

var logger = hclog.Default()
var cf = &config.Config{
	Logger:       logger,
	TurboVersion: "test",
	ApiUrl:       "api-url",
	LoginUrl:     "login-url",
}

type testResult struct {
	clientErr          error
	userConfigWritten  *config.TurborepoConfig
	repoConfigWritten  *config.TurborepoConfig
	clientTokenWritten string
	openedURL          string
	stepCh             chan struct{}
	client             dummyClient
}

func (tr *testResult) Deps() loginDeps {
	urlOpener := func(url string) error {
		tr.openedURL = url
		tr.stepCh <- struct{}{}
		return nil
	}
	return loginDeps{
		ui:      ui.Default(),
		openURL: urlOpener,
		client:  &tr.client,
		writeUserConfig: func(cf *config.TurborepoConfig) error {
			tr.userConfigWritten = cf
			return nil
		},
		writeRepoConfig: func(cf *config.TurborepoConfig) error {
			tr.repoConfigWritten = cf
			return nil
		},
	}
}

func newTest(redirectedURL string) *testResult {
	stepCh := make(chan struct{}, 1)
	tr := &testResult{
		stepCh: stepCh,
	}
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
		tr.stepCh <- struct{}{}
	}()
	return tr
}

func Test_run(t *testing.T) {
	test := newTest("http://127.0.0.1:9789/?token=my-token")
	err := run(cf, test.Deps())
	if err != nil {
		t.Errorf("expected to succeed, got error %v", err)
	}
	<-test.stepCh
	if test.clientErr != nil {
		t.Errorf("test client had error %v", test.clientErr)
	}

	expectedURL := "login-url/turborepo/token?redirect_uri=http://127.0.0.1:9789"
	if test.openedURL != expectedURL {
		t.Errorf("openedURL got %v, want %v", test.openedURL, expectedURL)
	}

	if test.userConfigWritten.Token != "my-token" {
		t.Errorf("config token got %v, want my-token", test.userConfigWritten.Token)
	}
	if test.client.setToken != "my-token" {
		t.Errorf("user client token got %v, want my-token", test.client.setToken)
	}
}

func Test_sso(t *testing.T) {
	redirectParams := make(url.Values)
	redirectParams.Add("token", "verification-token")
	redirectParams.Add("email", "test@example.com")
	test := newTest("http://127.0.0.1:9789/?" + redirectParams.Encode())
	err := loginSSO(cf, "my-team", test.Deps())
	if err != nil {
		t.Errorf("expected to succeed, got error %v", err)
	}
	<-test.stepCh
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
	if test.userConfigWritten.Token != expectedToken {
		t.Errorf("user config token got %v want %v", test.userConfigWritten.Token, expectedToken)
	}
	expectedTeamID := "sso-team-id"
	if test.repoConfigWritten.TeamId != expectedTeamID {
		t.Errorf("repo config team id got %v want %v", test.repoConfigWritten.TeamId, expectedTeamID)
	}
	if test.repoConfigWritten.Token != "" {
		t.Errorf("repo config file token, got %v want empty string", test.repoConfigWritten.Token)
	}
}
