package cacheitem

import (
	"encoding/hex"
	"io/fs"
	"os"
	"runtime"
	"testing"

	"github.com/vercel/turborepo/cli/internal/turbopath"
	"gotest.tools/v3/assert"
)

type createFileDefinition struct {
	Path     turbopath.AnchoredSystemPath
	Linkname string
	fs.FileMode
}

func createEntry(t *testing.T, anchor turbopath.AbsoluteSystemPath, fileDefinition createFileDefinition) error {
	t.Helper()
	if fileDefinition.FileMode.IsDir() {
		return createDir(t, anchor, fileDefinition)
	} else if fileDefinition.FileMode&os.ModeSymlink != 0 {
		return createSymlink(t, anchor, fileDefinition)
	} else if fileDefinition.FileMode&os.ModeNamedPipe != 0 {
		return createFifo(t, anchor, fileDefinition)
	} else {
		return createFile(t, anchor, fileDefinition)
	}
}

func createDir(t *testing.T, anchor turbopath.AbsoluteSystemPath, fileDefinition createFileDefinition) error {
	t.Helper()
	path := fileDefinition.Path.RestoreAnchor(anchor)
	mkdirAllErr := path.MkdirAllMode(fileDefinition.FileMode & 0777)
	assert.NilError(t, mkdirAllErr, "MkdirAll")
	return mkdirAllErr
}
func createFile(t *testing.T, anchor turbopath.AbsoluteSystemPath, fileDefinition createFileDefinition) error {
	t.Helper()
	path := fileDefinition.Path.RestoreAnchor(anchor)
	writeErr := path.WriteFile([]byte("file contents"), fileDefinition.FileMode&0777)
	assert.NilError(t, writeErr, "WriteFile")
	return writeErr
}
func createSymlink(t *testing.T, anchor turbopath.AbsoluteSystemPath, fileDefinition createFileDefinition) error {
	t.Helper()
	path := fileDefinition.Path.RestoreAnchor(anchor)
	symlinkErr := path.Symlink(fileDefinition.Linkname)
	assert.NilError(t, symlinkErr, "Symlink")
	lchmodErr := path.Lchmod(fileDefinition.FileMode & 0777)
	assert.NilError(t, lchmodErr, "Lchmod")
	return symlinkErr
}

func TestCreate(t *testing.T) {
	tests := []struct {
		name        string
		files       []createFileDefinition
		wantDarwin  string
		wantUnix    string
		wantWindows string
		wantErr     error
	}{
		{
			name: "hello world",
			files: []createFileDefinition{
				{
					Path:     turbopath.AnchoredSystemPath("hello world.txt"),
					FileMode: 0 | 0644,
				},
			},
			wantDarwin:  "4f39f1cab23906f3b89f313392ef7c26f2586e1c15fa6b577cce640c4781d082817927b4875a5413bc23e1248f0b198218998d70e7336e8b1244542ba446ca07",
			wantUnix:    "4f39f1cab23906f3b89f313392ef7c26f2586e1c15fa6b577cce640c4781d082817927b4875a5413bc23e1248f0b198218998d70e7336e8b1244542ba446ca07",
			wantWindows: "37a271d277c299cfe130ccfdb98af6e5909ade7a640a126d1495a57af1b1aed0676eedd2f0c918a9dfc04145051f52c783e7e6c0eb9aaa32af8238b47aed16bf",
		},
		{
			name: "links",
			files: []createFileDefinition{
				{
					Path:     turbopath.AnchoredSystemPath("one"),
					Linkname: "two",
					FileMode: 0 | os.ModeSymlink | 0777,
				},
				{
					Path:     turbopath.AnchoredSystemPath("two"),
					Linkname: "three",
					FileMode: 0 | os.ModeSymlink | 0777,
				},
				{
					Path:     turbopath.AnchoredSystemPath("three"),
					Linkname: "real",
					FileMode: 0 | os.ModeSymlink | 0777,
				},
				{
					Path:     turbopath.AnchoredSystemPath("real"),
					FileMode: 0 | 0644,
				},
			},
			wantDarwin:  "07278fdf37db4b212352367f391377bd6bac8f361dd834ae5522d809539bcf3b34d046873c1b45876d7372251446bb12c32f9fa9824914c4a1a01f6d7a206702",
			wantUnix:    "07278fdf37db4b212352367f391377bd6bac8f361dd834ae5522d809539bcf3b34d046873c1b45876d7372251446bb12c32f9fa9824914c4a1a01f6d7a206702",
			wantWindows: "46f4e6053867da99065e758c05648d2c5025830bf1e2fc9d54af1835e1d9ef3359f9a0ec942a4c8f88ebe427460cc92d2d9661956787b6045eb7ac9ecab4b5be",
		},
		{
			name: "subdirectory",
			files: []createFileDefinition{
				{
					Path:     turbopath.AnchoredSystemPath("parent"),
					FileMode: 0 | os.ModeDir | 0755,
				},
				{
					Path:     turbopath.AnchoredSystemPath("parent/child"),
					FileMode: 0 | 0644,
				},
			},
			wantDarwin:  "b513eea231daa84245d1d23d99fc398ccf17166ca49754ffbdcc1a3269cd75b7ad176a9c7095ff2481f71dca9fc350189747035f13d53b3a864e4fe35165233f",
			wantUnix:    "b513eea231daa84245d1d23d99fc398ccf17166ca49754ffbdcc1a3269cd75b7ad176a9c7095ff2481f71dca9fc350189747035f13d53b3a864e4fe35165233f",
			wantWindows: "59201a55277cf9182d3513110eae0391c3881e441fcb9ec7a22d4d1e7e4c640568b29fa1ece502791ab15a1415a21e861a36c5b93c9544d675e71f0d3a613909",
		},
		{
			name: "symlink permissions",
			files: []createFileDefinition{
				{
					Path:     turbopath.AnchoredSystemPath("one"),
					Linkname: "two",
					FileMode: 0 | os.ModeSymlink | 0644,
				},
			},
			wantDarwin:  "3ea9d8a4581a0c2ba77557c72447b240c5ac622edcdac570a0bf597c276c2917b4ea73e6c373bbac593a480e396845651fa4b51e049531ff5d44c0adb807c2d9",
			wantUnix:    "c07abb37f1bcf96e1edf5e1c45d58186475d1451eb0cc0fb906a7cef013800d5005855be1998da067c67a6f8a27c7187d7eeafd2a50ad93f8088d9f44e2202e7",
			wantWindows: "8d83add4152804c50bafa2779160cbd93d4e4d29deffa48526600291ba0b973c3a56e6adcc1eaa1e26dd18c352929279341db088841eccfa27e3ab37916961da",
		},
		{
			name: "unsupported types error",
			files: []createFileDefinition{
				{
					Path:     turbopath.AnchoredSystemPath("fifo"),
					FileMode: 0 | os.ModeNamedPipe | 0644,
				},
			},
			wantErr: errUnsupportedFileType,
		},
	}
	for _, tt := range tests {
		t.Run(tt.name, func(t *testing.T) {
			inputDir := turbopath.AbsoluteSystemPath(t.TempDir())
			archiveDir := turbopath.AbsoluteSystemPath(t.TempDir())
			archivePath := turbopath.AnchoredSystemPath("out.tar.zst").RestoreAnchor(archiveDir)

			cacheItem, cacheCreateErr := Create(archivePath)
			assert.NilError(t, cacheCreateErr, "Cache Create")

			for _, file := range tt.files {
				createErr := createEntry(t, inputDir, file)
				if createErr != nil {
					assert.ErrorIs(t, createErr, tt.wantErr)
					assert.NilError(t, cacheItem.Close(), "Close")
					return
				}

				addFileError := cacheItem.AddFile(inputDir, file.Path)
				if addFileError != nil {
					assert.ErrorIs(t, addFileError, tt.wantErr)
					assert.NilError(t, cacheItem.Close(), "Close")
					return
				}
			}

			assert.NilError(t, cacheItem.Close(), "Cache Close")

			openedCacheItem, openedCacheItemErr := Open(archivePath)
			assert.NilError(t, openedCacheItemErr, "Cache Open")

			// We actually only need to compare the generated SHA.
			// That ensures we got the same output. (Effectively snapshots.)
			// This must be called after `Close` because both `tar` and `gzip` have footers.
			shaOne, shaOneErr := openedCacheItem.GetSha()
			assert.NilError(t, shaOneErr, "GetSha")
			snapshot := hex.EncodeToString(shaOne)

			switch runtime.GOOS {
			case "darwin":
				assert.Equal(t, snapshot, tt.wantDarwin, "Got expected hash.")
			case "windows":
				assert.Equal(t, snapshot, tt.wantWindows, "Got expected hash.")
			default:
				assert.Equal(t, snapshot, tt.wantUnix, "Got expected hash.")
			}
			assert.NilError(t, openedCacheItem.Close(), "Close")
		})
	}
}
