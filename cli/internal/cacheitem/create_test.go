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
			want: "ac50a36fbd1c77ebe270bb1a383da5b1a5cf546bf9e04682ff4b2691daca5e8f16f878d6a3db179a2d1c363b4fadc98ce80645a6f820b5b399b5ac0a3c07a384",
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
			want: "048053cbfe2b8dc316c9ce99d0d12f3902c2d4512e323f40a2775b777383eabb00e12488189b569285af09571810b0a34b144f9cec3bb88f1452f7c0e29e95aa",
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
			want: "b8919559a95f229b9d0a460882566fee5cdd824388ecb6ef1a65938d1172ca1678ea054a0079a93ab58f041a78e3f35c911ed622a8d6c39d768299aa7f349cfa",
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
