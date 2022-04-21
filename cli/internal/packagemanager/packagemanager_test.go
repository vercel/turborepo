package packagemanager

import (
	"testing"

	"github.com/vercel/turborepo/cli/internal/fs"
)

func TestParsePackageManagerString(t *testing.T) {
	tests := []struct {
		name           string
		packageManager string
		wantManager    string
		wantVersion    string
		wantErr        bool
	}{
		{
			name:           "errors with a tag version",
			packageManager: "npm@latest",
			wantManager:    "",
			wantVersion:    "",
			wantErr:        true,
		},
		{
			name:           "errors with no version",
			packageManager: "npm",
			wantManager:    "",
			wantVersion:    "",
			wantErr:        true,
		},
		{
			name:           "requires fully-qualified semver versions (one digit)",
			packageManager: "npm@1",
			wantManager:    "",
			wantVersion:    "",
			wantErr:        true,
		},
		{
			name:           "requires fully-qualified semver versions (two digits)",
			packageManager: "npm@1.2",
			wantManager:    "",
			wantVersion:    "",
			wantErr:        true,
		},
		{
			name:           "supports custom labels",
			packageManager: "npm@1.2.3-alpha.1",
			wantManager:    "npm",
			wantVersion:    "1.2.3-alpha.1",
			wantErr:        false,
		},
		{
			name:           "only supports specified package managers",
			packageManager: "pip@1.2.3",
			wantManager:    "",
			wantVersion:    "",
			wantErr:        true,
		},
		{
			name:           "supports npm",
			packageManager: "npm@0.0.1",
			wantManager:    "npm",
			wantVersion:    "0.0.1",
			wantErr:        false,
		},
		{
			name:           "supports pnpm",
			packageManager: "pnpm@0.0.1",
			wantManager:    "pnpm",
			wantVersion:    "0.0.1",
			wantErr:        false,
		},
		{
			name:           "supports yarn",
			packageManager: "yarn@111.0.1",
			wantManager:    "yarn",
			wantVersion:    "111.0.1",
			wantErr:        false,
		},
	}
	for _, tt := range tests {
		t.Run(tt.name, func(t *testing.T) {
			gotManager, gotVersion, err := ParsePackageManagerString(tt.packageManager)
			if (err != nil) != tt.wantErr {
				t.Errorf("ParsePackageManagerString() error = %v, wantErr %v", err, tt.wantErr)
				return
			}
			if gotManager != tt.wantManager {
				t.Errorf("ParsePackageManagerString() got manager = %v, want manager %v", gotManager, tt.wantManager)
			}
			if gotVersion != tt.wantVersion {
				t.Errorf("ParsePackageManagerString() got version = %v, want version %v", gotVersion, tt.wantVersion)
			}
		})
	}
}

func TestGetPackageManager(t *testing.T) {
	tests := []struct {
		name             string
		projectDirectory string
		pkg              *fs.PackageJSON
		want             string
		wantErr          bool
	}{
		{
			name:             "finds npm from a package manager string",
			projectDirectory: "",
			pkg:              &fs.PackageJSON{PackageManager: "npm@1.2.3"},
			want:             "nodejs-npm",
			wantErr:          false,
		},
		{
			name:             "finds pnpm from a package manager string",
			projectDirectory: "",
			pkg:              &fs.PackageJSON{PackageManager: "pnpm@1.2.3"},
			want:             "nodejs-pnpm",
			wantErr:          false,
		},
		{
			name:             "finds yarn from a package manager string",
			projectDirectory: "",
			pkg:              &fs.PackageJSON{PackageManager: "yarn@1.2.3"},
			want:             "nodejs-yarn",
			wantErr:          false,
		},
		{
			name:             "finds berry from a package manager string",
			projectDirectory: "",
			pkg:              &fs.PackageJSON{PackageManager: "yarn@2.3.4"},
			want:             "nodejs-berry",
			wantErr:          false,
		},
		// TODO: Test discovery of the package manager from filesystem and command execution after adopting afero
	}
	for _, tt := range tests {
		t.Run(tt.name, func(t *testing.T) {
			gotPackageManager, err := GetPackageManager(tt.projectDirectory, tt.pkg)
			if (err != nil) != tt.wantErr {
				t.Errorf("GetPackageManager() error = %v, wantErr %v", err, tt.wantErr)
				return
			}
			if gotPackageManager.Name != tt.want {
				t.Errorf("GetPackageManager() = %v, want %v", gotPackageManager.Name, tt.want)
			}
		})
	}
}

func Test_readPackageManager(t *testing.T) {
	tests := []struct {
		name    string
		pkg     *fs.PackageJSON
		want    string
		wantErr bool
	}{
		{
			name:    "finds npm from a package manager string",
			pkg:     &fs.PackageJSON{PackageManager: "npm@1.2.3"},
			want:    "nodejs-npm",
			wantErr: false,
		},
		{
			name:    "finds pnpm from a package manager string",
			pkg:     &fs.PackageJSON{PackageManager: "pnpm@1.2.3"},
			want:    "nodejs-pnpm",
			wantErr: false,
		},
		{
			name:    "finds yarn from a package manager string",
			pkg:     &fs.PackageJSON{PackageManager: "yarn@1.2.3"},
			want:    "nodejs-yarn",
			wantErr: false,
		},
		{
			name:    "finds berry from a package manager string",
			pkg:     &fs.PackageJSON{PackageManager: "yarn@2.3.4"},
			want:    "nodejs-berry",
			wantErr: false,
		},
	}
	for _, tt := range tests {
		t.Run(tt.name, func(t *testing.T) {
			gotPackageManager, err := readPackageManager(tt.pkg)
			if (err != nil) != tt.wantErr {
				t.Errorf("readPackageManager() error = %v, wantErr %v", err, tt.wantErr)
				return
			}
			if gotPackageManager.Name != tt.want {
				t.Errorf("GetPackageManager() = %v, want %v", gotPackageManager.Name, tt.want)
			}
		})
	}
}
