package fs

import (
	"errors"
	"fmt"
	"os"
	"path/filepath"
	"reflect"
	"strings"
	"testing"

	"github.com/vercel/turborepo/cli/internal/turbopath"
)

func getFixture(id int) turbopath.AbsoluteSystemPath {
	cwd, _ := os.Getwd()
	root := filepath.VolumeName(cwd) + string(os.PathSeparator)
	checking := cwd

	for checking != root {
		fixtureDirectory := filepath.Join(checking, "fixtures")
		_, err := os.Stat(fixtureDirectory)
		if !errors.Is(err, os.ErrNotExist) {
			// Found the fixture directory!
			files, _ := os.ReadDir(fixtureDirectory)

			// Grab the specified fixture.
			for _, file := range files {
				fileName := file.Name()
				if strings.Index(fileName, fmt.Sprintf("%02d-", id)) == 0 {
					return turbopath.AbsoluteSystemPath(filepath.Join(fixtureDirectory, fileName))
				}
			}
		}
		checking = filepath.Join(checking, "..")
	}

	panic("fixtures not found!")
}

func Test_gitHashObject(t *testing.T) {
	fixturePath := getFixture(1)
	traversePath, err := getTraversePath(AbsolutePath(fixturePath))
	if err != nil {
		return
	}

	tests := []struct {
		name        string
		rootPath    turbopath.AbsoluteSystemPath
		filesToHash []turbopath.RelativeUnixPath
		want        map[turbopath.RelativeUnixPath]string
		wantErr     bool
	}{
		{
			name:        "No paths",
			rootPath:    fixturePath,
			filesToHash: []turbopath.RelativeUnixPath{},
			want:        map[turbopath.RelativeUnixPath]string{},
		},
		{
			name:     "Special characters",
			rootPath: fixturePath,
			filesToHash: []turbopath.RelativeUnixPath{
				turbopath.RelativeUnixPath("new\nline"),
				turbopath.RelativeUnixPath("\"quote\""),
			},
			want: map[turbopath.RelativeUnixPath]string{
				"new\nline": "e69de29bb2d1d6434b8b29ae775ad8c2e48c5391",
				"\"quote\"": "e69de29bb2d1d6434b8b29ae775ad8c2e48c5391",
			},
		},
		{
			name:     "Absolute paths come back relative to rootPath",
			rootPath: fixturePath.Join("child"),
			filesToHash: []turbopath.RelativeUnixPath{
				turbopath.RelativeUnixPath("../root.json"),
				turbopath.RelativeUnixPath("child.json"),
				turbopath.RelativeUnixPath("grandchild/grandchild.json"),
			},
			want: map[turbopath.RelativeUnixPath]string{
				"../root.json":               "e69de29bb2d1d6434b8b29ae775ad8c2e48c5391",
				"child.json":                 "e69de29bb2d1d6434b8b29ae775ad8c2e48c5391",
				"grandchild/grandchild.json": "e69de29bb2d1d6434b8b29ae775ad8c2e48c5391",
			},
		},
		{
			name:     "Traverse outside of the repo",
			rootPath: fixturePath.Join(traversePath.ToRelativeSystemPath()).Join(".."),
			filesToHash: []turbopath.RelativeUnixPath{
				turbopath.RelativeUnixPath("null.json"),
			},
			want:    nil,
			wantErr: true,
		},
		{
			name:     "Nonexistent file",
			rootPath: fixturePath,
			filesToHash: []turbopath.RelativeUnixPath{
				turbopath.RelativeUnixPath("nonexistent.json"),
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
