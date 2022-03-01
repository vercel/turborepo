package login

import (
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

	ch := make(chan struct{})
	openedURL := ""
	urlOpener := func(url string) error {
		openedURL = url
		ch <- struct{}{}
		return nil
	}

	// When we get the ping, send a token
	go func() {
		<-ch
		http.Get("http://127.0.0.1:9789/?token=my-token")
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
