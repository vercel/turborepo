package hashing

import (
	"errors"
	"fmt"
	"os"
	"os/exec"
	"path/filepath"
	"reflect"
	"runtime"
	"strings"
	"testing"

	"github.com/vercel/turbo/cli/internal/fs"
	"github.com/vercel/turbo/cli/internal/turbopath"
	"gotest.tools/v3/assert"
)

func getFixture(id int) turbopath.AbsoluteSystemPath {
	cwd, _ := os.Getwd()
	root := turbopath.AbsoluteSystemPath(filepath.VolumeName(cwd) + string(os.PathSeparator))
	checking := turbopath.AbsoluteSystemPath(cwd)

	for checking != root {
		fixtureDirectory := checking.Join("fixtures")
		_, err := os.Stat(fixtureDirectory.ToString())
		if !errors.Is(err, os.ErrNotExist) {
			// Found the fixture directory!
			files, _ := os.ReadDir(fixtureDirectory.ToString())

			// Grab the specified fixture.
			for _, file := range files {
				fileName := turbopath.RelativeSystemPath(file.Name())
				if strings.Index(fileName.ToString(), fmt.Sprintf("%02d-", id)) == 0 {
					return turbopath.AbsoluteSystemPath(fixtureDirectory.Join(fileName))
				}
			}
		}
		checking = checking.Join("..")
	}

	panic("fixtures not found!")
}

func TestSpecialCharacters(t *testing.T) {
	if runtime.GOOS == "windows" {
		return
	}

	fixturePath := getFixture(1)
	newlinePath := turbopath.AnchoredUnixPath("new\nline").ToSystemPath()
	quotePath := turbopath.AnchoredUnixPath("\"quote\"").ToSystemPath()
	newline := newlinePath.RestoreAnchor(fixturePath)
	quote := quotePath.RestoreAnchor(fixturePath)

	// Setup
	one := os.WriteFile(newline.ToString(), []byte{}, 0644)
	two := os.WriteFile(quote.ToString(), []byte{}, 0644)

	// Cleanup
	defer func() {
		one := os.Remove(newline.ToString())
		two := os.Remove(quote.ToString())

		if one != nil || two != nil {
			return
		}
	}()

	// Setup error check
	if one != nil || two != nil {
		return
	}

	tests := []struct {
		name        string
		rootPath    turbopath.AbsoluteSystemPath
		filesToHash []turbopath.AnchoredSystemPath
		want        map[turbopath.AnchoredUnixPath]string
		wantErr     bool
	}{
		{
			name:     "Quotes",
			rootPath: fixturePath,
			filesToHash: []turbopath.AnchoredSystemPath{
				quotePath,
			},
			want: map[turbopath.AnchoredUnixPath]string{
				quotePath.ToUnixPath(): "e69de29bb2d1d6434b8b29ae775ad8c2e48c5391",
			},
		},
		{
			name:     "Newlines",
			rootPath: fixturePath,
			filesToHash: []turbopath.AnchoredSystemPath{
				newlinePath,
			},
			want: map[turbopath.AnchoredUnixPath]string{
				newlinePath.ToUnixPath(): "e69de29bb2d1d6434b8b29ae775ad8c2e48c5391",
			},
		},
	}
	for _, tt := range tests {
		t.Run(tt.name, func(t *testing.T) {
			got, err := gitHashObject(tt.rootPath, tt.filesToHash)
			if (err != nil) != tt.wantErr {
				t.Errorf("gitHashObject() error = %v, wantErr %v", err, tt.wantErr)
				return
			}
			if !reflect.DeepEqual(got, tt.want) {
				t.Errorf("gitHashObject() = %v, want %v", got, tt.want)
			}
		})
	}
}

func Test_gitHashObject(t *testing.T) {
	fixturePath := getFixture(1)
	traversePath, err := getTraversePath(fixturePath)
	if err != nil {
		return
	}

	tests := []struct {
		name        string
		rootPath    turbopath.AbsoluteSystemPath
		filesToHash []turbopath.AnchoredSystemPath
		want        map[turbopath.AnchoredUnixPath]string
		wantErr     bool
	}{
		{
			name:        "No paths",
			rootPath:    fixturePath,
			filesToHash: []turbopath.AnchoredSystemPath{},
			want:        map[turbopath.AnchoredUnixPath]string{},
		},
		{
			name:     "Absolute paths come back relative to rootPath",
			rootPath: fixturePath.Join("child"),
			filesToHash: []turbopath.AnchoredSystemPath{
				turbopath.AnchoredUnixPath("../root.json").ToSystemPath(),
				turbopath.AnchoredUnixPath("child.json").ToSystemPath(),
				turbopath.AnchoredUnixPath("grandchild/grandchild.json").ToSystemPath(),
			},
			want: map[turbopath.AnchoredUnixPath]string{
				"../root.json":               "e69de29bb2d1d6434b8b29ae775ad8c2e48c5391",
				"child.json":                 "e69de29bb2d1d6434b8b29ae775ad8c2e48c5391",
				"grandchild/grandchild.json": "e69de29bb2d1d6434b8b29ae775ad8c2e48c5391",
			},
		},
		{
			name:     "Traverse outside of the repo",
			rootPath: fixturePath.Join(traversePath.ToSystemPath(), ".."),
			filesToHash: []turbopath.AnchoredSystemPath{
				turbopath.AnchoredUnixPath("null.json").ToSystemPath(),
			},
			want:    nil,
			wantErr: true,
		},
		{
			name:     "Nonexistent file",
			rootPath: fixturePath,
			filesToHash: []turbopath.AnchoredSystemPath{
				turbopath.AnchoredUnixPath("nonexistent.json").ToSystemPath(),
			},
			want:    nil,
			wantErr: true,
		},
	}
	for _, tt := range tests {
		t.Run(tt.name, func(t *testing.T) {
			got, err := gitHashObject(tt.rootPath, tt.filesToHash)
			if (err != nil) != tt.wantErr {
				t.Errorf("gitHashObject() error = %v, wantErr %v", err, tt.wantErr)
				return
			}
			if !reflect.DeepEqual(got, tt.want) {
				t.Errorf("gitHashObject() = %v, want %v", got, tt.want)
			}
		})
	}
}

func Test_getTraversePath(t *testing.T) {
	fixturePath := getFixture(1)

	tests := []struct {
		name     string
		rootPath turbopath.AbsoluteSystemPath
		want     turbopath.RelativeUnixPath
		wantErr  bool
	}{
		{
			name:     "From fixture location",
			rootPath: fixturePath,
			want:     turbopath.RelativeUnixPath("../../../"),
			wantErr:  false,
		},
		{
			name:     "Traverse out of git repo",
			rootPath: fixturePath.UntypedJoin("..", "..", "..", ".."),
			want:     "",
			wantErr:  true,
		},
	}
	for _, tt := range tests {
		t.Run(tt.name, func(t *testing.T) {
			got, err := getTraversePath(tt.rootPath)
			if (err != nil) != tt.wantErr {
				t.Errorf("getTraversePath() error = %v, wantErr %v", err, tt.wantErr)
				return
			}
			if !reflect.DeepEqual(got, tt.want) {
				t.Errorf("getTraversePath() = %v, want %v", got, tt.want)
			}
		})
	}
}

func requireGitCmd(t *testing.T, repoRoot turbopath.AbsoluteSystemPath, args ...string) {
	t.Helper()
	cmd := exec.Command("git", args...)
	cmd.Dir = repoRoot.ToString()
	out, err := cmd.CombinedOutput()
	if err != nil {
		t.Fatalf("git commit failed: %v %v", err, string(out))
	}
}

func TestGetPackageDeps(t *testing.T) {
	// Directory structure:
	// <root>/
	//   new-root-file <- new file not added to git
	//   my-pkg/
	//     committed-file
	//     deleted-file
	//     uncommitted-file <- new file not added to git
	//     dir/
	//       nested-file

	repoRoot := fs.AbsoluteSystemPathFromUpstream(t.TempDir())
	myPkgDir := repoRoot.UntypedJoin("my-pkg")

	// create the dir first
	err := myPkgDir.MkdirAll(0775)
	assert.NilError(t, err, "CreateDir")

	// create file 1
	committedFilePath := myPkgDir.UntypedJoin("committed-file")
	err = committedFilePath.WriteFile([]byte("committed bytes"), 0644)
	assert.NilError(t, err, "WriteFile")

	// create file 2
	deletedFilePath := myPkgDir.UntypedJoin("deleted-file")
	err = deletedFilePath.WriteFile([]byte("delete-me"), 0644)
	assert.NilError(t, err, "WriteFile")

	// create file 3
	nestedPath := myPkgDir.UntypedJoin("dir", "nested-file")
	assert.NilError(t, nestedPath.EnsureDir(), "EnsureDir")
	assert.NilError(t, nestedPath.WriteFile([]byte("nested"), 0644), "WriteFile")

	// create a package.json
	packageJSONPath := myPkgDir.UntypedJoin("package.json")
	err = packageJSONPath.WriteFile([]byte("{}"), 0644)
	assert.NilError(t, err, "WriteFile")

	// set up git repo and commit all
	requireGitCmd(t, repoRoot, "init", ".")
	requireGitCmd(t, repoRoot, "config", "--local", "user.name", "test")
	requireGitCmd(t, repoRoot, "config", "--local", "user.email", "test@example.com")
	requireGitCmd(t, repoRoot, "add", ".")
	requireGitCmd(t, repoRoot, "commit", "-m", "foo")

	// remove a file
	err = deletedFilePath.Remove()
	assert.NilError(t, err, "Remove")

	// create another untracked file in git
	uncommittedFilePath := myPkgDir.UntypedJoin("uncommitted-file")
	err = uncommittedFilePath.WriteFile([]byte("uncommitted bytes"), 0644)
	assert.NilError(t, err, "WriteFile")

	// create an untracked file in git up a level
	rootFilePath := repoRoot.UntypedJoin("new-root-file")
	err = rootFilePath.WriteFile([]byte("new-root bytes"), 0644)
	assert.NilError(t, err, "WriteFile")

	tests := []struct {
		opts     *PackageDepsOptions
		expected map[turbopath.AnchoredUnixPath]string
	}{
		// base case. when inputs aren't specified, all files hashes are computed
		{
			opts: &PackageDepsOptions{
				PackagePath: "my-pkg",
			},
			expected: map[turbopath.AnchoredUnixPath]string{
				"committed-file":   "3a29e62ea9ba15c4a4009d1f605d391cdd262033",
				"uncommitted-file": "4e56ad89387e6379e4e91ddfe9872cf6a72c9976",
				"package.json":     "9e26dfeeb6e641a33dae4961196235bdb965b21b",
				"dir/nested-file":  "bfe53d766e64d78f80050b73cd1c88095bc70abb",
			},
		},
		// with inputs, only the specified inputs are hashed
		{
			opts: &PackageDepsOptions{
				PackagePath:   "my-pkg",
				InputPatterns: []string{"uncommitted-file"},
			},
			expected: map[turbopath.AnchoredUnixPath]string{
				"package.json":     "9e26dfeeb6e641a33dae4961196235bdb965b21b",
				"uncommitted-file": "4e56ad89387e6379e4e91ddfe9872cf6a72c9976",
			},
		},
		// inputs with glob pattern also works
		{
			opts: &PackageDepsOptions{
				PackagePath:   "my-pkg",
				InputPatterns: []string{"**/*-file"},
			},
			expected: map[turbopath.AnchoredUnixPath]string{
				"committed-file":   "3a29e62ea9ba15c4a4009d1f605d391cdd262033",
				"uncommitted-file": "4e56ad89387e6379e4e91ddfe9872cf6a72c9976",
				"package.json":     "9e26dfeeb6e641a33dae4961196235bdb965b21b",
				"dir/nested-file":  "bfe53d766e64d78f80050b73cd1c88095bc70abb",
			},
		},
		// inputs with traversal work
		{
			opts: &PackageDepsOptions{
				PackagePath:   "my-pkg",
				InputPatterns: []string{"../**/*-file"},
			},
			expected: map[turbopath.AnchoredUnixPath]string{
				"../new-root-file": "8906ddcdd634706188bd8ef1c98ac07b9be3425e",
				"committed-file":   "3a29e62ea9ba15c4a4009d1f605d391cdd262033",
				"uncommitted-file": "4e56ad89387e6379e4e91ddfe9872cf6a72c9976",
				"package.json":     "9e26dfeeb6e641a33dae4961196235bdb965b21b",
				"dir/nested-file":  "bfe53d766e64d78f80050b73cd1c88095bc70abb",
			},
		},
		// inputs with another glob pattern works
		{
			opts: &PackageDepsOptions{
				PackagePath:   "my-pkg",
				InputPatterns: []string{"**/{uncommitted,committed}-file"},
			},
			expected: map[turbopath.AnchoredUnixPath]string{
				"committed-file":   "3a29e62ea9ba15c4a4009d1f605d391cdd262033",
				"package.json":     "9e26dfeeb6e641a33dae4961196235bdb965b21b",
				"uncommitted-file": "4e56ad89387e6379e4e91ddfe9872cf6a72c9976",
			},
		},
		// inputs with another glob pattern + traversal work
		{
			opts: &PackageDepsOptions{
				PackagePath:   "my-pkg",
				InputPatterns: []string{"../**/{new-root,uncommitted,committed}-file"},
			},
			expected: map[turbopath.AnchoredUnixPath]string{
				"../new-root-file": "8906ddcdd634706188bd8ef1c98ac07b9be3425e",
				"committed-file":   "3a29e62ea9ba15c4a4009d1f605d391cdd262033",
				"package.json":     "9e26dfeeb6e641a33dae4961196235bdb965b21b",
				"uncommitted-file": "4e56ad89387e6379e4e91ddfe9872cf6a72c9976",
			},
		},
	}
	for _, tt := range tests {
		got, err := GetPackageDeps(repoRoot, tt.opts)
		if err != nil {
			t.Errorf("GetPackageDeps got error %v", err)
			continue
		}
		assert.DeepEqual(t, got, tt.expected)
	}
}

func Test_memoizedGetTraversePath(t *testing.T) {
	fixturePath := getFixture(1)

	gotOne, _ := memoizedGetTraversePath(fixturePath)
	gotTwo, _ := memoizedGetTraversePath(fixturePath)

	assert.Check(t, gotOne == gotTwo, "The strings are identical.")
}
