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
			wantUnix:    "ac50a36fbd1c77ebe270bb1a383da5b1a5cf546bf9e04682ff4b2691daca5e8f16f878d6a3db179a2d1c363b4fadc98ce80645a6f820b5b399b5ac0a3c07a384",
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
			wantUnix:    "3ef6504edc2865b89afe7aa07c181425c79a7f4193786792bc56a58c70cfc9cf4b8486f8af868c58894ba05ea2133893ad6a0de5d1f488cd0c5ad2ca8fc96204",
			wantWindows: "59201a55277cf9182d3513110eae0391c3881e441fcb9ec7a22d4d1e7e4c640568b29fa1ece502791ab15a1415a21e861a36c5b93c9544d675e71f0d3a613909",
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
			// These are the same because the privileges for directories by default end up being 0755 on both.
			wantUnix:    "b8919559a95f229b9d0a460882566fee5cdd824388ecb6ef1a65938d1172ca1678ea054a0079a93ab58f041a78e3f35c911ed622a8d6c39d768299aa7f349cfa",
			wantWindows: "b8919559a95f229b9d0a460882566fee5cdd824388ecb6ef1a65938d1172ca1678ea054a0079a93ab58f041a78e3f35c911ed622a8d6c39d768299aa7f349cfa",
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

			if runtime.GOOS == "windows" {
				assert.Equal(t, snapshot, tt.wantWindows, "Got expected hash.")
			} else {
				assert.Equal(t, snapshot, tt.wantUnix, "Got expected hash.")
			}
			assert.Equal(t, snapshot, snapshotTwo, "Reopened snapshot matches.")
		})
	}
}
