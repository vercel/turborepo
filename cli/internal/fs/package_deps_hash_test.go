package fs

import (
	"errors"
	"fmt"
	"os"
	"path/filepath"
	"reflect"
	"strings"
	"testing"
)

func getFixture(id int) AbsoluteSystemPath {
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
					return AbsoluteSystemPath(filepath.Join(fixtureDirectory, fileName))
				}
			}
		}
		checking = filepath.Join(checking, "..")
	}

	panic("fixtures not found!")
}

func Test_gitHashObject(t *testing.T) {
	fixturePath := getFixture(1)
	tests := []struct {
		name        string
		rootPath    AbsoluteSystemPath
		filesToHash []RelativeUnixPath
		want        map[RelativeUnixPath]string
		wantErr     bool
	}{
		{
			name:        "No paths",
			rootPath:    fixturePath,
			filesToHash: []RelativeUnixPath{},
			want:        map[RelativeUnixPath]string{},
		},
		{
			name:     "Special characters",
			rootPath: fixturePath,
			filesToHash: []RelativeUnixPath{
				AbsoluteSystemPath(fixturePath.Join("new\nline").ToString()),
				AbsoluteSystemPath(fixturePath.Join("\"quote\"").ToString()),
			},
			want: map[RelativeUnixPath]string{
				"new\nline": "e69de29bb2d1d6434b8b29ae775ad8c2e48c5391",
				"\"quote\"": "e69de29bb2d1d6434b8b29ae775ad8c2e48c5391",
			},
		},
		{
			name:     "Absolute paths come back relative to rootPath",
			rootPath: fixturePath.Join("child"),
			filesToHash: []RelativeUnixPath{
				AbsoluteSystemPath(fixturePath.Join("root.json").ToString()),
				AbsoluteSystemPath(fixturePath.Join("child", "child.json").ToString()),
				AbsoluteSystemPath(fixturePath.Join("child", "grandchild", "grandchild.json").ToString()),
			},
			want: map[RelativeUnixPath]string{
				"../root.json":               "e69de29bb2d1d6434b8b29ae775ad8c2e48c5391",
				"child.json":                 "e69de29bb2d1d6434b8b29ae775ad8c2e48c5391",
				"grandchild/grandchild.json": "e69de29bb2d1d6434b8b29ae775ad8c2e48c5391",
			},
		},
		{
			name:     "RelativeSystemPath inputs are relative to rootPath",
			rootPath: fixturePath.Join(),
			filesToHash: []RelativeUnixPath{
				RelativeSystemPath(filepath.Join("root.json")),
				RelativeSystemPath(filepath.Join("child", "child.json")),
				RelativeSystemPath(filepath.Join("child", "grandchild", "grandchild.json")),
			},
			want: map[RelativeUnixPath]string{
				"root.json":                        "e69de29bb2d1d6434b8b29ae775ad8c2e48c5391",
				"child/child.json":                 "e69de29bb2d1d6434b8b29ae775ad8c2e48c5391",
				"child/grandchild/grandchild.json": "e69de29bb2d1d6434b8b29ae775ad8c2e48c5391",
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
