package login

import (
	"fmt"
	"net/http"
	"testing"

	"github.com/hashicorp/go-hclog"
	"github.com/vercel/turborepo/cli/internal/client"
	"github.com/vercel/turborepo/cli/internal/config"
	"github.com/vercel/turborepo/cli/internal/ui"
)

type dummyClient struct {
	setToken string
}

func (d *dummyClient) SetToken(t string) {
	d.setToken = t
}

func (d *dummyClient) GetUser() (*client.UserResponse, error) {
	return &client.UserResponse{}, nil
}

func Test_run(t *testing.T) {
	logger := hclog.Default()
	cf := &config.Config{
		Logger:       logger,
		TurboVersion: "test",
		ApiUrl:       "api-url",
		LoginUrl:     "login-url",
	}

	ch := make(chan struct{}, 1)
	openedURL := ""
	urlOpener := func(url string) error {
		openedURL = url
		ch <- struct{}{}
		return nil
	}

	// When we get the ping, send a token
	var clientErr error
	go func() {
		<-ch
		client := &http.Client{
			CheckRedirect: func(req *http.Request, via []*http.Request) error {
				return http.ErrUseLastResponse
			},
		}
		resp, err := client.Get("http://127.0.0.1:9789/?token=my-token")
		if err != nil {
			clientErr = err
		} else if resp != nil && resp.StatusCode != http.StatusFound {
			clientErr = fmt.Errorf("invalid status %v", resp.StatusCode)
		}
		ch <- struct{}{}
	}()

	var writtenConfig *config.TurborepoConfig
	writeConfig := func(cf *config.TurborepoConfig) error {
		writtenConfig = cf
		return nil
	}

	client := &dummyClient{}
	err := run(cf, loginDeps{
		openURL:     urlOpener,
		ui:          ui.Default(),
		writeConfig: writeConfig,
		client:      client,
	})
	if err != nil {
		t.Errorf("expected to succeed, got error %v", err)
	}
	<-ch
	if clientErr != nil {
		t.Errorf("test client had error %v", clientErr)
	}

	expectedURL := "login-url/turborepo/token?redirect_uri=http://127.0.0.1:9789"
	if openedURL != expectedURL {
		t.Errorf("openedURL got %v, want %v", openedURL, expectedURL)
	}

	if writtenConfig.Token != "my-token" {
		t.Errorf("config token got %v, want my-token", writtenConfig.Token)
	}
	if client.setToken != "my-token" {
		t.Errorf("user client token got %v, want my-token", client.setToken)
	}
}
