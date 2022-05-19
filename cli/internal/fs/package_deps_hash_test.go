package fs

import (
	"os"
	"path/filepath"
	"reflect"
	"strings"
	"testing"
)

func Test_GetHashableDeps(t *testing.T) {
	cwd, err := os.Getwd()
	if err != nil {
		t.Fatalf("failed to get cwd %v", err)
	}
	cliDir, err := filepath.Abs(filepath.Join(cwd, "..", ".."))
	if err != nil {
		t.Fatalf("failed to get cli dir: %v", err)
	}
	if filepath.Base(cliDir) != "cli" {
		t.Fatalf("did not find cli dir, found %v", cliDir)
	}
	turboPath := filepath.Join(cliDir, "..", "turbo.json")
	makefilePath := filepath.Join(cliDir, "Makefile")
	mainPath := filepath.Join(cliDir, "cmd", "turbo", "main.go")
	hashes, err := GetHashableDeps([]string{turboPath, makefilePath, mainPath}, cliDir)
	if err != nil {
		t.Fatalf("failed to hash files: %v", err)
	}
	// Note that the paths here are platform independent, so hardcoded slashes should be fine
	expected := []string{
		"../turbo.json",
		"Makefile",
		"cmd/turbo/main.go",
	}
	for _, key := range expected {
		if _, ok := hashes[key]; !ok {
			t.Errorf("hashes missing %v", key)
		}
	}
	if len(hashes) != len(expected) {
		keys := []string{}
		for key := range hashes {
			keys = append(keys, key)
		}
		t.Errorf("hashes mismatch. got %v want %v", strings.Join(keys, ", "), strings.Join(expected, ", "))
	}
}

func TestGetPackageDeps(t *testing.T) {
	type args struct {
		repoRoot AbsolutePath
		p        *PackageDepsOptions
	}
	tests := []struct {
		name    string
		args    args
		want    map[RepoRelativeUnixPath]string
		wantErr bool
	}{
		{
			name: "Simple",
			args: args{
				repoRoot: UnsafeToAbsolutePath("/Users/nathanhammond/repos/vercel/turborepo"),
				p: &PackageDepsOptions{
					PackagePath: "cli",
				},
			},
			want:    map[RepoRelativeUnixPath]string{},
			wantErr: false,
		},
	}
	for _, tt := range tests {
		t.Run(tt.name, func(t *testing.T) {
			got, err := GetPackageDeps(tt.args.repoRoot, tt.args.p)
			if (err != nil) != tt.wantErr {
				t.Errorf("GetPackageDeps() error = %v, wantErr %v", err, tt.wantErr)
				return
			}
			if !reflect.DeepEqual(got, tt.want) {
				t.Errorf("GetPackageDeps() = %v, want %v", got, tt.want)
			}
		})
	}
}
