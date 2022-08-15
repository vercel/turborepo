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

	"github.com/vercel/turborepo/cli/internal/fs"
	"github.com/vercel/turborepo/cli/internal/turbopath"
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
	newlinePath := turbopath.AnchoredSystemPath("new\nline")
	quotePath := turbopath.AnchoredSystemPath("\"quote\"")
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
				turbopath.AnchoredSystemPath(quotePath),
			},
			want: map[turbopath.AnchoredUnixPath]string{
				quotePath.ToUnixPath(): "e69de29bb2d1d6434b8b29ae775ad8c2e48c5391",
			},
		},
		{
			name:     "Newlines",
			rootPath: fixturePath,
			filesToHash: []turbopath.AnchoredSystemPath{
				turbopath.AnchoredSystemPath(newlinePath),
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
				turbopath.AnchoredSystemPath(filepath.Join("..", "root.json")),
				turbopath.AnchoredSystemPath("child.json"),
				turbopath.AnchoredSystemPath(filepath.Join("grandchild", "grandchild.json")),
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
				turbopath.AnchoredSystemPath("null.json"),
			},
			want:    nil,
			wantErr: true,
		},
		{
			name:     "Nonexistent file",
			rootPath: fixturePath,
			filesToHash: []turbopath.AnchoredSystemPath{
				turbopath.AnchoredSystemPath("nonexistent.json"),
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
			rootPath: fixturePath.Join("..", "..", "..", ".."),
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

func requireGitCmd(t *testing.T, repoRoot fs.AbsolutePath, args ...string) {
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
	//   my-pkg/
	//     committed-file
	//     deleted-file
	//     uncommitted-file <- new file not added to git
	//     dir/
	//       nested-file

	repoRoot := fs.AbsolutePathFromUpstream(t.TempDir())
	myPkgDir := repoRoot.Join("my-pkg")
	committedFilePath := myPkgDir.Join("committed-file")
	err := committedFilePath.EnsureDir()
	assert.NilError(t, err, "EnsureDir")
	err = committedFilePath.WriteFile([]byte("committed bytes"), 0644)
	assert.NilError(t, err, "WriteFile")
	deletedFilePath := myPkgDir.Join("deleted-file")
	err = deletedFilePath.WriteFile([]byte("delete-me"), 0644)
	assert.NilError(t, err, "WriteFile")
	nestedPath := myPkgDir.Join("dir", "nested-file")
	assert.NilError(t, nestedPath.EnsureDir(), "EnsureDir")
	assert.NilError(t, nestedPath.WriteFile([]byte("nested"), 0644), "WriteFile")
	requireGitCmd(t, repoRoot, "init", ".")
	requireGitCmd(t, repoRoot, "config", "--local", "user.name", "test")
	requireGitCmd(t, repoRoot, "config", "--local", "user.email", "test@example.com")
	requireGitCmd(t, repoRoot, "add", ".")
	requireGitCmd(t, repoRoot, "commit", "-m", "foo")
	err = deletedFilePath.Remove()
	assert.NilError(t, err, "Remove")
	uncommittedFilePath := myPkgDir.Join("uncommitted-file")
	err = uncommittedFilePath.WriteFile([]byte("uncommitted bytes"), 0644)
	assert.NilError(t, err, "WriteFile")

	tests := []struct {
		opts     *PackageDepsOptions
		expected map[turbopath.AnchoredUnixPath]string
	}{
		{
			opts: &PackageDepsOptions{
				PackagePath: "my-pkg",
			},
			expected: map[turbopath.AnchoredUnixPath]string{
				"committed-file":   "3a29e62ea9ba15c4a4009d1f605d391cdd262033",
				"uncommitted-file": "4e56ad89387e6379e4e91ddfe9872cf6a72c9976",
				"dir/nested-file":  "bfe53d766e64d78f80050b73cd1c88095bc70abb",
			},
		},
		{
			opts: &PackageDepsOptions{
				PackagePath:   "my-pkg",
				InputPatterns: []string{"uncommitted-file"},
			},
			expected: map[turbopath.AnchoredUnixPath]string{
				"uncommitted-file": "4e56ad89387e6379e4e91ddfe9872cf6a72c9976",
			},
		},
		{
			opts: &PackageDepsOptions{
				PackagePath:   "my-pkg",
				InputPatterns: []string{"**/*-file"},
			},
			expected: map[turbopath.AnchoredUnixPath]string{
				"committed-file":   "3a29e62ea9ba15c4a4009d1f605d391cdd262033",
				"uncommitted-file": "4e56ad89387e6379e4e91ddfe9872cf6a72c9976",
				"dir/nested-file":  "bfe53d766e64d78f80050b73cd1c88095bc70abb",
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
