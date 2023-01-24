package cacheitem

import (
	"encoding/hex"
	"io/fs"
	"os"
	"runtime"
	"testing"

	"github.com/vercel/turbo/cli/internal/turbopath"
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
			wantWindows: "e304d1ba8c51209f97bd11dabf27ca06996b70a850db592343942c49480de47bcbb4b7131fb3dd4d7564021d3bc0e648919e4876572b46ac1da97fca92b009c5",
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
			wantWindows: "d4dac527e40860ee1ba3fdf2b9b12a1eba385050cf4f5877558dd531f0ecf2a06952fd5f88b852ad99e010943ed7b7f1437b727796369524e85f0c06f25d62c9",
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
			wantWindows: "a8c3cba54e4dc214d3b21c3fa284d4032fe317d2f88943159efd5d16f3551ab53fae5c92ebf8acdd1bdb85d1238510b7938772cb11a0daa1b72b5e0f2700b5c7",
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
			wantUnix:    "99d953cbe1c0d8545e6f8382208fcefe14bcbefe39872f7b6310da14ac195b9a1b04b6d7b4b56f01a27216176193344a92488f99e124fcd68693f313f7137a1c",
			wantWindows: "a4b1dc5c296f8ac4c9124727c1d84d70f72872c7bb4ced6d83ee312889e822baf1eaa72f88e624fb1aac4339d0a1f766ede77eabd2e4524eb26e89f883dc479d",
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
		getTestFunc := func(compressed bool) func(t *testing.T) {
			return func(t *testing.T) {
				inputDir := turbopath.AbsoluteSystemPath(t.TempDir())
				archiveDir := turbopath.AbsoluteSystemPath(t.TempDir())
				var archivePath turbopath.AbsoluteSystemPath
				if compressed {
					archivePath = turbopath.AnchoredSystemPath("out.tar.zst").RestoreAnchor(archiveDir)
				} else {
					archivePath = turbopath.AnchoredSystemPath("out.tar").RestoreAnchor(archiveDir)
				}

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

				// We only check for repeatability on compressed caches.
				if compressed {
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
				}
			}
		}
		t.Run(tt.name, getTestFunc(false))
		t.Run(tt.name+"zst", getTestFunc(true))
	}
}
