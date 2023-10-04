package ci

import (
	"os"
	"reflect"
	"strings"
	"testing"
)

func getVendor(name string) Vendor {
	for _, v := range Vendors {
		if v.Name == name {
			return v
		}
	}
	return Vendor{}
}

func TestInfo(t *testing.T) {
	tests := []struct {
		name   string
		setEnv []string
		want   Vendor
	}{
		{
			name:   "AppVeyor",
			setEnv: []string{"APPVEYOR"},
			want:   getVendor("AppVeyor"),
		},
		{
			name:   "Vercel",
			setEnv: []string{"VERCEL", "NOW_BUILDER"},
			want:   getVendor("Vercel"),
		},
		{
			name:   "Render",
			setEnv: []string{"RENDER"},
			want:   getVendor("Render"),
		},
		{
			name:   "Netlify",
			setEnv: []string{"NETLIFY"},
			want:   getVendor("Netlify CI"),
		},
		{
			name:   "Jenkins",
			setEnv: []string{"BUILD_ID", "JENKINS_URL"},
			want:   getVendor("Jenkins"),
		},
		{
			name:   "Jenkins - failing",
			setEnv: []string{"BUILD_ID"},
			want:   getVendor(""),
		},
		{
			name:   "GitHub Actions",
			setEnv: []string{"GITHUB_ACTIONS"},
			want:   getVendor("GitHub Actions"),
		},
		{
			name:   "Codeship",
			setEnv: []string{"CI_NAME=codeship"},
			want:   getVendor("Codeship"),
		},
	}
	for _, tt := range tests {
		t.Run(tt.name, func(t *testing.T) {
			// unset existing envs
			liveCi := ""
			if Name() == "GitHub Actions" {
				liveCi = os.Getenv("GITHUB_ACTIONS")
				err := os.Unsetenv("GITHUB_ACTIONS")
				if err != nil {
					t.Errorf("Error un-setting GITHUB_ACTIONS env: %s", err)
				}
			}
			// set envs
			for _, env := range tt.setEnv {
				envParts := strings.Split(env, "=")
				val := "some value"
				if len(envParts) > 1 {
					val = envParts[1]
				}
				err := os.Setenv(envParts[0], val)
				if err != nil {
					t.Errorf("Error setting %s for %s test", envParts[0], tt.name)
				}
				defer os.Unsetenv(envParts[0]) //nolint errcheck

			}
			// run test
			if got := Info(); !reflect.DeepEqual(got, tt.want) {
				t.Errorf("Info() = %v, want %v", got, tt.want)
			}

			// reset env
			if Name() == "GitHub Actions" {
				err := os.Setenv("GITHUB_ACTIONS", liveCi)
				if err != nil {
					t.Errorf("Error re-setting GITHUB_ACTIONS env: %s", err)
				}
			}
		})
	}
}
