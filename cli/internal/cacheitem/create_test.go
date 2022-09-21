package cacheitem

import (
	"encoding/hex"
	"io/fs"
	"os"
	"runtime"
	"syscall"
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
	mkdirAllErr := path.MkdirAll()
	assert.NilError(t, mkdirAllErr, "MkdirAll")
	return mkdirAllErr
}
func createFile(t *testing.T, anchor turbopath.AbsoluteSystemPath, fileDefinition createFileDefinition) error {
	t.Helper()
	path := fileDefinition.Path.RestoreAnchor(anchor)
	writeErr := path.WriteFile([]byte("file contents"), 0666)
	assert.NilError(t, writeErr, "WriteFile")
	return writeErr
}
func createSymlink(t *testing.T, anchor turbopath.AbsoluteSystemPath, fileDefinition createFileDefinition) error {
	t.Helper()
	path := fileDefinition.Path.RestoreAnchor(anchor)
	symlinkErr := path.Symlink(fileDefinition.Linkname)
	assert.NilError(t, symlinkErr, "Symlink")
	return symlinkErr
}
func createFifo(t *testing.T, anchor turbopath.AbsoluteSystemPath, fileDefinition createFileDefinition) error {
	t.Helper()
	path := fileDefinition.Path.RestoreAnchor(anchor)
	if runtime.GOOS != "windows" {
		fifoErr := syscall.Mknod(path.ToString(), syscall.S_IFIFO|0666, 0)
		assert.NilError(t, fifoErr, "FIFO")
		return fifoErr
	}

	return errUnsupportedFileType
}

func TestCreate(t *testing.T) {
	tests := []struct {
		name    string
		files   []createFileDefinition
		want    string
		wantErr error
	}{
		{
			name: "hello world",
			files: []createFileDefinition{
				{
					Path: turbopath.AnchoredSystemPath("hello world.txt"),
				},
			},
			want: "6abf8eaf63e6e943c02562c002336342fad89502dce42b71eeee22e7318aa2884bb14a4ac55f2acc0bc75f55626cec50d3f1552810b9bbe2dbc39e2d656a73e2",
		},
		{
			name: "links",
			files: []createFileDefinition{
				{
					Path:     turbopath.AnchoredSystemPath("one"),
					Linkname: "two",
					FileMode: 0 | os.ModeSymlink,
				},
				{
					Path:     turbopath.AnchoredSystemPath("two"),
					Linkname: "three",
					FileMode: 0 | os.ModeSymlink,
				},
				{
					Path:     turbopath.AnchoredSystemPath("three"),
					Linkname: "real",
					FileMode: 0 | os.ModeSymlink,
				},
				{
					Path: turbopath.AnchoredSystemPath("real"),
				},
			},
			want: "e197b0a937d48d4358dc842063569a30d77c6de99bb3a48fe771afc71b1013ccde20437b86d6a5ac6906f24f41c712a8dba17e0bca82265f45477e2779d1b913",
		},
		{
			name: "subdirectory",
			files: []createFileDefinition{
				{
					Path:     turbopath.AnchoredSystemPath("parent"),
					FileMode: 0 | os.ModeDir,
				},
				{
					Path: turbopath.AnchoredSystemPath("parent/child"),
				},
			},
			want: "d3f21380181e0ceef216d13a7d61faac2efc97e81fed2c0ed8540dffbaf85797d494dfff12c6781dbeff95c8771babeb9d0c86c847e089dfd9fbf231e5d4275c",
		},
		{
			name: "unsupported types error",
			files: []createFileDefinition{
				{
					Path:     turbopath.AnchoredSystemPath("fifo"),
					FileMode: 0 | os.ModeNamedPipe,
				},
			},
			wantErr: errUnsupportedFileType,
		},
	}
	for _, tt := range tests {
		t.Run(tt.name, func(t *testing.T) {
			inputDir := turbopath.AbsoluteSystemPath(t.TempDir())
			archiveDir := turbopath.AbsoluteSystemPath(t.TempDir())
			archivePath := turbopath.AnchoredSystemPath("out.tar.gz").RestoreAnchor(archiveDir)

			cacheItem, cacheCreateErr := Create(archivePath)
			assert.NilError(t, cacheCreateErr, "Cache Create")

			for _, file := range tt.files {
				createErr := createEntry(t, inputDir, file)
				if createErr != nil {
					assert.ErrorIs(t, createErr, tt.wantErr)
					return
				}

				addFileError := cacheItem.AddFile(inputDir, file.Path)
				if addFileError != nil {
					assert.ErrorIs(t, addFileError, tt.wantErr)
					return
				}
			}

			closeErr := cacheItem.Close()
			assert.NilError(t, closeErr, "Cache Close")

			// We actually only need to compare the generated SHA.
			// That ensures we got the same output. (Effectively snapshots.)
			// This must be called after `Close` because both `tar` and `gzip` have footers.
			snapshot := hex.EncodeToString(cacheItem.GetSha())

			openedCacheItem, openedCacheItemErr := Open(archivePath)
			assert.NilError(t, openedCacheItemErr, "Cache Open")
			snapshotTwo := hex.EncodeToString(openedCacheItem.GetSha())

			assert.Equal(t, snapshot, tt.want, "Got expected hash.")
			assert.Equal(t, snapshot, snapshotTwo, "Reopened snapshot matches.")
		})
	}
}
