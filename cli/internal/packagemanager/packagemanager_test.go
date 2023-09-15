package packagemanager

import (
	"os"
	"path/filepath"
	"reflect"
	"sort"
	"testing"

	"github.com/vercel/turbo/cli/internal/fs"
	"github.com/vercel/turbo/cli/internal/turbopath"
	"gotest.tools/v3/assert"
)

func Test_GetWorkspaces(t *testing.T) {
	type test struct {
		name     string
		pm       PackageManager
		rootPath turbopath.AbsoluteSystemPath
		want     []string
		wantErr  bool
	}

	cwd, _ := os.Getwd()

	repoRoot, err := fs.GetCwd(cwd)
	assert.NilError(t, err, "GetCwd")
	rootPath := map[string]turbopath.AbsoluteSystemPath{
		"nodejs-bun":   repoRoot.UntypedJoin("../../../examples/with-yarn"),
		"nodejs-npm":   repoRoot.UntypedJoin("../../../examples/with-yarn"),
		"nodejs-berry": repoRoot.UntypedJoin("../../../examples/with-yarn"),
		"nodejs-yarn":  repoRoot.UntypedJoin("../../../examples/with-yarn"),
		"nodejs-pnpm":  repoRoot.UntypedJoin("../../../examples/basic"),
		"nodejs-pnpm6": repoRoot.UntypedJoin("../../../examples/basic"),
	}

	want := map[string][]string{
		"nodejs-bun": {
			filepath.ToSlash(filepath.Join(cwd, "../../../examples/with-yarn/apps/docs/package.json")),
			filepath.ToSlash(filepath.Join(cwd, "../../../examples/with-yarn/apps/web/package.json")),
			filepath.ToSlash(filepath.Join(cwd, "../../../examples/with-yarn/packages/eslint-config-custom/package.json")),
			filepath.ToSlash(filepath.Join(cwd, "../../../examples/with-yarn/packages/tsconfig/package.json")),
			filepath.ToSlash(filepath.Join(cwd, "../../../examples/with-yarn/packages/ui/package.json")),
		},
		"nodejs-npm": {
			filepath.ToSlash(filepath.Join(cwd, "../../../examples/with-yarn/apps/docs/package.json")),
			filepath.ToSlash(filepath.Join(cwd, "../../../examples/with-yarn/apps/web/package.json")),
			filepath.ToSlash(filepath.Join(cwd, "../../../examples/with-yarn/packages/eslint-config-custom/package.json")),
			filepath.ToSlash(filepath.Join(cwd, "../../../examples/with-yarn/packages/tsconfig/package.json")),
			filepath.ToSlash(filepath.Join(cwd, "../../../examples/with-yarn/packages/ui/package.json")),
		},
		"nodejs-berry": {
			filepath.ToSlash(filepath.Join(cwd, "../../../examples/with-yarn/apps/docs/package.json")),
			filepath.ToSlash(filepath.Join(cwd, "../../../examples/with-yarn/apps/web/package.json")),
			filepath.ToSlash(filepath.Join(cwd, "../../../examples/with-yarn/packages/eslint-config-custom/package.json")),
			filepath.ToSlash(filepath.Join(cwd, "../../../examples/with-yarn/packages/tsconfig/package.json")),
			filepath.ToSlash(filepath.Join(cwd, "../../../examples/with-yarn/packages/ui/package.json")),
		},
		"nodejs-yarn": {
			filepath.ToSlash(filepath.Join(cwd, "../../../examples/with-yarn/apps/docs/package.json")),
			filepath.ToSlash(filepath.Join(cwd, "../../../examples/with-yarn/apps/web/package.json")),
			filepath.ToSlash(filepath.Join(cwd, "../../../examples/with-yarn/packages/eslint-config-custom/package.json")),
			filepath.ToSlash(filepath.Join(cwd, "../../../examples/with-yarn/packages/tsconfig/package.json")),
			filepath.ToSlash(filepath.Join(cwd, "../../../examples/with-yarn/packages/ui/package.json")),
		},
		"nodejs-pnpm": {
			filepath.ToSlash(filepath.Join(cwd, "../../../examples/basic/apps/docs/package.json")),
			filepath.ToSlash(filepath.Join(cwd, "../../../examples/basic/apps/web/package.json")),
			filepath.ToSlash(filepath.Join(cwd, "../../../examples/basic/packages/eslint-config-custom/package.json")),
			filepath.ToSlash(filepath.Join(cwd, "../../../examples/basic/packages/tsconfig/package.json")),
			filepath.ToSlash(filepath.Join(cwd, "../../../examples/basic/packages/ui/package.json")),
		},
		"nodejs-pnpm6": {
			filepath.ToSlash(filepath.Join(cwd, "../../../examples/basic/apps/docs/package.json")),
			filepath.ToSlash(filepath.Join(cwd, "../../../examples/basic/apps/web/package.json")),
			filepath.ToSlash(filepath.Join(cwd, "../../../examples/basic/packages/eslint-config-custom/package.json")),
			filepath.ToSlash(filepath.Join(cwd, "../../../examples/basic/packages/tsconfig/package.json")),
			filepath.ToSlash(filepath.Join(cwd, "../../../examples/basic/packages/ui/package.json")),
		},
	}

	tests := make([]test, len(packageManagers))
	for i, packageManager := range packageManagers {
		tests[i] = test{
			name:     packageManager.Name,
			pm:       packageManager,
			rootPath: rootPath[packageManager.Name],
			want:     want[packageManager.Name],
			wantErr:  false,
		}
	}

	for _, tt := range tests {
		t.Run(tt.name, func(t *testing.T) {
			gotWorkspaces, err := tt.pm.GetWorkspaces(tt.rootPath)

			gotToSlash := make([]string, len(gotWorkspaces))
			for index, workspace := range gotWorkspaces {
				gotToSlash[index] = filepath.ToSlash(workspace)
			}

			if (err != nil) != tt.wantErr {
				t.Errorf("GetWorkspaces() error = %v, wantErr %v", err, tt.wantErr)
				return
			}
			sort.Strings(gotToSlash)
			if !reflect.DeepEqual(gotToSlash, tt.want) {
				t.Errorf("GetWorkspaces() = %v, want %v", gotToSlash, tt.want)
			}
		})
	}
}

func Test_GetWorkspaceIgnores(t *testing.T) {
	type test struct {
		name     string
		pm       PackageManager
		rootPath turbopath.AbsoluteSystemPath
		want     []string
		wantErr  bool
	}

	cwdRaw, err := os.Getwd()
	assert.NilError(t, err, "os.Getwd")
	cwd, err := fs.GetCwd(cwdRaw)
	assert.NilError(t, err, "GetCwd")
	want := map[string][]string{
		"nodejs-bun":   {"**/node_modules", "**/.git"},
		"nodejs-npm":   {"**/node_modules/**"},
		"nodejs-berry": {"**/node_modules", "**/.git", "**/.yarn"},
		"nodejs-yarn":  {"apps/*/node_modules/**", "packages/*/node_modules/**"},
		"nodejs-pnpm":  {"**/node_modules/**", "**/bower_components/**", "packages/skip"},
		"nodejs-pnpm6": {"**/node_modules/**", "**/bower_components/**", "packages/skip"},
	}

	tests := make([]test, len(packageManagers))
	for i, packageManager := range packageManagers {
		tests[i] = test{
			name:     packageManager.Name,
			pm:       packageManager,
			rootPath: cwd.UntypedJoin("fixtures"),
			want:     want[packageManager.Name],
			wantErr:  false,
		}
	}

	for _, tt := range tests {
		t.Run(tt.name, func(t *testing.T) {
			gotWorkspaceIgnores, err := tt.pm.GetWorkspaceIgnores(tt.rootPath)

			gotToSlash := make([]string, len(gotWorkspaceIgnores))
			for index, ignore := range gotWorkspaceIgnores {
				gotToSlash[index] = filepath.ToSlash(ignore)
			}

			if (err != nil) != tt.wantErr {
				t.Errorf("GetWorkspaceIgnores() error = %v, wantErr %v", err, tt.wantErr)
				return
			}
			if !reflect.DeepEqual(gotToSlash, tt.want) {
				t.Errorf("GetWorkspaceIgnores() = %v, want %v", gotToSlash, tt.want)
			}
		})
	}
}
