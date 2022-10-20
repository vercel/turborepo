package turbopath

import (
	"os"
	"runtime"
	"testing"

	"gotest.tools/v3/assert"
	"gotest.tools/v3/fs"
)

func Test_Mkdir(t *testing.T) {
	type Case struct {
		name         string
		isDir        bool
		exists       bool
		mode         os.FileMode
		expectedMode os.FileMode
	}

	cases := []Case{
		{
			name:         "dir doesn't exist",
			exists:       false,
			expectedMode: os.ModeDir | 0777,
		},
		{
			name:         "path exists as file",
			exists:       true,
			isDir:        false,
			mode:         0666,
			expectedMode: os.ModeDir | 0755,
		},
		{
			name:         "dir exists with incorrect mode",
			exists:       true,
			isDir:        false,
			mode:         os.ModeDir | 0755,
			expectedMode: os.ModeDir | 0655,
		},
		{
			name:         "dir exists with correct mode",
			exists:       true,
			isDir:        false,
			mode:         os.ModeDir | 0755,
			expectedMode: os.ModeDir | 0755,
		},
	}

	for _, testCase := range cases {
		testDir := fs.NewDir(t, "system-path-mkdir-test")
		testName := testCase.name
		path := testDir.Join("foo")
		if testCase.isDir {
			err := os.Mkdir(path, testCase.mode)
			assert.NilError(t, err, "%s: Mkdir", testName)
		} else if testCase.exists {
			file, err := os.Create(path)
			assert.NilError(t, err, "%s: Create", testName)
			err = file.Chmod(testCase.mode)
			assert.NilError(t, err, "%s: Chmod", testName)
			err = file.Close()
			assert.NilError(t, err, "%s: Close", testName)
		}

		testPath := AbsoluteSystemPath(path)
		err := testPath.MkdirAllMode(testCase.expectedMode)
		assert.NilError(t, err, "%s: Mkdir", testName)

		stat, err := testPath.Lstat()
		assert.NilError(t, err, "%s: Lstat", testName)
		assert.Assert(t, stat.IsDir(), testName)

		assert.Assert(t, stat.IsDir(), testName)

		if runtime.GOOS == "windows" {
			// For windows os.Chmod will only change the writable bit so that's all we check
			assert.Equal(t, stat.Mode().Perm()&0200, testCase.expectedMode.Perm()&0200, testName)
		} else {
			assert.Equal(t, stat.Mode(), testCase.expectedMode, testName)
		}

	}
}

func TestAbsoluteSystemPath_Findup(t *testing.T) {
	tests := []struct {
		name     string
		p        AbsoluteSystemPath
		fileName RelativeSystemPath
		want     AbsoluteSystemPath
		wantErr  bool
	}{
		// TODO: Add test cases.
	}
	for _, tt := range tests {
		t.Run(tt.name, func(t *testing.T) {
			got, err := tt.p.Findup(tt.fileName)
			if (err != nil) != tt.wantErr {
				t.Errorf("AbsoluteSystemPath.Findup() error = %v, wantErr %v", err, tt.wantErr)
				return
			}
			if got != tt.want {
				t.Errorf("AbsoluteSystemPath.Findup() = %v, want %v", got, tt.want)
			}
		})
	}
}
