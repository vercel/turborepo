package packagemanager

import (
	"os"
	"testing"

	"github.com/vercel/turbo/cli/internal/fs"
	"github.com/vercel/turbo/cli/internal/turbopath"
	"gotest.tools/v3/assert"
)

func Test_CanPrune(t *testing.T) {
	type test struct {
		name     string
		pm       PackageManager
		rootPath turbopath.AbsoluteSystemPath
		want     bool
		wantErr  bool
	}

	type want struct {
		want    bool
		wantErr bool
	}

	cwdRaw, err := os.Getwd()
	assert.NilError(t, err, "os.Getwd")
	cwd, err := fs.GetCwd(cwdRaw)
	assert.NilError(t, err, "GetCwd")
	wants := map[string]want{
		"nodejs-npm":   {true, false},
		"nodejs-berry": {false, true},
		"nodejs-yarn":  {true, false},
		"nodejs-pnpm":  {true, false},
		"nodejs-pnpm6": {true, false},
	}

	tests := make([]test, len(packageManagers))
	for i, packageManager := range packageManagers {
		tests[i] = test{
			name:     packageManager.Name,
			pm:       packageManager,
			rootPath: cwd.UntypedJoin("../../../examples/with-yarn"),
			want:     wants[packageManager.Name].want,
			wantErr:  wants[packageManager.Name].wantErr,
		}
	}

	for _, tt := range tests {
		t.Run(tt.name, func(t *testing.T) {
			canPrune, err := tt.pm.CanPrune(tt.rootPath)

			if (err != nil) != tt.wantErr {
				t.Errorf("CanPrune() error = %v, wantErr %v", err, tt.wantErr)
				return
			}
			if canPrune != tt.want {
				t.Errorf("CanPrune() = %v, want %v", canPrune, tt.want)
			}
		})
	}
}
