package cacheitem

import (
	"archive/tar"
	"errors"
	"fmt"
	"io"
	"io/fs"
	"os"
	"path/filepath"
	"reflect"
	"runtime"
	"syscall"
	"testing"

	"github.com/DataDog/zstd"
	"github.com/vercel/turborepo/cli/internal/turbopath"
	"gotest.tools/v3/assert"
)

type tarFile struct {
	Body string
	*tar.Header
}

type restoreFile struct {
	Name     turbopath.AnchoredUnixPath
	Linkname string
	fs.FileMode
}

// generateTar is used specifically to generate tar files that Turborepo would
// rarely or never encounter without malicious or pathological inputs. We use it
// to make sure that we respond well in these scenarios during restore attempts.
func generateTar(t *testing.T, files []tarFile) turbopath.AbsoluteSystemPath {
	t.Helper()
	testDir := turbopath.AbsoluteSystemPath(t.TempDir())
	testArchivePath := testDir.UntypedJoin("out.tar")

	handle, handleCreateErr := testArchivePath.Create()
	assert.NilError(t, handleCreateErr, "os.Create")

	tw := tar.NewWriter(handle)

	for _, file := range files {
		if file.Header.Typeflag == tar.TypeReg {
			file.Header.Size = int64(len(file.Body))
		}

		writeHeaderErr := tw.WriteHeader(file.Header)
		assert.NilError(t, writeHeaderErr, "tw.WriteHeader")

		_, writeErr := tw.Write([]byte(file.Body))
		assert.NilError(t, writeErr, "tw.Write")
	}

	twCloseErr := tw.Close()
	assert.NilError(t, twCloseErr, "tw.Close")

	handleCloseErr := handle.Close()
	assert.NilError(t, handleCloseErr, "handle.Close")

	return testArchivePath
}

// compressTar splits the compression of a tar file so that we don't
// accidentally diverge in tar creation while still being able to test
// restoration from tar and from .tar.zst.
func compressTar(t *testing.T, archivePath turbopath.AbsoluteSystemPath) turbopath.AbsoluteSystemPath {
	t.Helper()

	inputHandle, inputHandleOpenErr := archivePath.Open()
	assert.NilError(t, inputHandleOpenErr, "os.Open")

	outputPath := archivePath + ".zst"
	outputHandle, outputHandleCreateErr := outputPath.Create()
	assert.NilError(t, outputHandleCreateErr, "os.Create")

	zw := zstd.NewWriter(outputHandle)
	_, copyError := io.Copy(zw, inputHandle)
	assert.NilError(t, copyError, "io.Copy")

	zwCloseErr := zw.Close()
	assert.NilError(t, zwCloseErr, "zw.Close")

	inputHandleCloseErr := inputHandle.Close()
	assert.NilError(t, inputHandleCloseErr, "inputHandle.Close")

	outputHandleCloseErr := outputHandle.Close()
	assert.NilError(t, outputHandleCloseErr, "outputHandle.Close")

	return outputPath
}

func generateAnchor(t *testing.T) turbopath.AbsoluteSystemPath {
	t.Helper()
	testDir := turbopath.AbsoluteSystemPath(t.TempDir())
	anchorPoint := testDir.UntypedJoin("anchor")

	mkdirErr := anchorPoint.Mkdir(0777)
	assert.NilError(t, mkdirErr, "Mkdir")

	return anchorPoint
}

func assertFileExists(t *testing.T, anchor turbopath.AbsoluteSystemPath, diskFile restoreFile) {
	t.Helper()
	// If we have gotten here we can assume this to be true.
	processedName := diskFile.Name.ToSystemPath()
	fullName := processedName.RestoreAnchor(anchor)
	fileInfo, err := fullName.Lstat()
	assert.NilError(t, err, "Lstat")

	assert.Equal(t, fileInfo.Mode()&fs.ModePerm, diskFile.FileMode&fs.ModePerm, "File has the expected permissions: "+processedName)
	assert.Equal(t, fileInfo.Mode()|fs.ModePerm, diskFile.FileMode|fs.ModePerm, "File has the expected mode.")

	if diskFile.FileMode&os.ModeSymlink != 0 {
		linkname, err := fullName.Readlink()
		assert.NilError(t, err, "Readlink")

		// We restore Linkname verbatim.
		assert.Equal(t, linkname, diskFile.Linkname, "Link target matches.")
	}
}

func TestOpen(t *testing.T) {
	type wantErr struct {
		unix    error
		windows error
	}
	type wantOutput struct {
		unix    []turbopath.AnchoredSystemPath
		windows []turbopath.AnchoredSystemPath
	}
	type wantFiles struct {
		unix    []restoreFile
		windows []restoreFile
	}
	tests := []struct {
		name       string
		tarFiles   []tarFile
		wantOutput wantOutput
		wantFiles  wantFiles
		wantErr    wantErr
	}{
		{
			name: "cache optimized",
			tarFiles: []tarFile{
				{
					Header: &tar.Header{
						Name:     "one/",
						Typeflag: tar.TypeDir,
						Mode:     0755,
					},
				},
				{
					Header: &tar.Header{
						Name:     "one/two/",
						Typeflag: tar.TypeDir,
						Mode:     0755,
					},
				},
				{
					Header: &tar.Header{
						Name:     "one/two/three/",
						Typeflag: tar.TypeDir,
						Mode:     0755,
					},
				},
				{
					Header: &tar.Header{
						Name:     "one/two/three/file-one",
						Typeflag: tar.TypeReg,
						Mode:     0644,
					},
				},
				{
					Header: &tar.Header{
						Name:     "one/two/three/file-two",
						Typeflag: tar.TypeReg,
						Mode:     0644,
					},
				},
				{
					Header: &tar.Header{
						Name:     "one/two/a/",
						Typeflag: tar.TypeDir,
						Mode:     0755,
					},
				},
				{
					Header: &tar.Header{
						Name:     "one/two/a/file",
						Typeflag: tar.TypeReg,
						Mode:     0644,
					},
				},
				{
					Header: &tar.Header{
						Name:     "one/two/b/",
						Typeflag: tar.TypeDir,
						Mode:     0755,
					},
				},
				{
					Header: &tar.Header{
						Name:     "one/two/b/file",
						Typeflag: tar.TypeReg,
						Mode:     0644,
					},
				},
			},
			wantFiles: wantFiles{
				unix: []restoreFile{
					{
						Name:     "one",
						FileMode: 0 | os.ModeDir | 0755,
					},
					{
						Name:     "one/two",
						FileMode: 0 | os.ModeDir | 0755,
					},
					{
						Name:     "one/two/three",
						FileMode: 0 | os.ModeDir | 0755,
					},
					{
						Name:     "one/two/three/file-one",
						FileMode: 0644,
					},
					{
						Name:     "one/two/three/file-two",
						FileMode: 0644,
					},
					{
						Name:     "one/two/a",
						FileMode: 0 | os.ModeDir | 0755,
					},
					{
						Name:     "one/two/a/file",
						FileMode: 0644,
					},
					{
						Name:     "one/two/b",
						FileMode: 0 | os.ModeDir | 0755,
					},
					{
						Name:     "one/two/b/file",
						FileMode: 0644,
					},
				},
				windows: []restoreFile{
					{
						Name:     "one",
						FileMode: 0 | os.ModeDir | 0777,
					},
					{
						Name:     "one/two",
						FileMode: 0 | os.ModeDir | 0777,
					},
					{
						Name:     "one/two/three",
						FileMode: 0 | os.ModeDir | 0777,
					},
					{
						Name:     "one/two/three/file-one",
						FileMode: 0666,
					},
					{
						Name:     "one/two/three/file-two",
						FileMode: 0666,
					},
					{
						Name:     "one/two/a",
						FileMode: 0 | os.ModeDir | 0777,
					},
					{
						Name:     "one/two/a/file",
						FileMode: 0666,
					},
					{
						Name:     "one/two/b",
						FileMode: 0 | os.ModeDir | 0777,
					},
					{
						Name:     "one/two/b/file",
						FileMode: 0666,
					},
				},
			},
			wantOutput: wantOutput{
				unix: turbopath.AnchoredUnixPathArray{
					"one",
					"one/two",
					"one/two/three",
					"one/two/three/file-one",
					"one/two/three/file-two",
					"one/two/a",
					"one/two/a/file",
					"one/two/b",
					"one/two/b/file",
				}.ToSystemPathArray(),
			},
		},
		{
			name: "pathological cache works",
			tarFiles: []tarFile{
				{
					Header: &tar.Header{
						Name:     "one/",
						Typeflag: tar.TypeDir,
						Mode:     0755,
					},
				},
				{
					Header: &tar.Header{
						Name:     "one/two/",
						Typeflag: tar.TypeDir,
						Mode:     0755,
					},
				},
				{
					Header: &tar.Header{
						Name:     "one/two/a/",
						Typeflag: tar.TypeDir,
						Mode:     0755,
					},
				},
				{
					Header: &tar.Header{
						Name:     "one/two/b/",
						Typeflag: tar.TypeDir,
						Mode:     0755,
					},
				},
				{
					Header: &tar.Header{
						Name:     "one/two/three/",
						Typeflag: tar.TypeDir,
						Mode:     0755,
					},
				},
				{
					Header: &tar.Header{
						Name:     "one/two/a/file",
						Typeflag: tar.TypeReg,
						Mode:     0644,
					},
				},
				{
					Header: &tar.Header{
						Name:     "one/two/b/file",
						Typeflag: tar.TypeReg,
						Mode:     0644,
					},
				},
				{
					Header: &tar.Header{
						Name:     "one/two/three/file-one",
						Typeflag: tar.TypeReg,
						Mode:     0644,
					},
				},
				{
					Header: &tar.Header{
						Name:     "one/two/three/file-two",
						Typeflag: tar.TypeReg,
						Mode:     0644,
					},
				},
			},
			wantFiles: wantFiles{
				unix: []restoreFile{
					{
						Name:     "one",
						FileMode: 0 | os.ModeDir | 0755,
					},
					{
						Name:     "one/two",
						FileMode: 0 | os.ModeDir | 0755,
					},
					{
						Name:     "one/two/three",
						FileMode: 0 | os.ModeDir | 0755,
					},
					{
						Name:     "one/two/three/file-one",
						FileMode: 0644,
					},
					{
						Name:     "one/two/three/file-two",
						FileMode: 0644,
					},
					{
						Name:     "one/two/a",
						FileMode: 0 | os.ModeDir | 0755,
					},
					{
						Name:     "one/two/a/file",
						FileMode: 0644,
					},
					{
						Name:     "one/two/b",
						FileMode: 0 | os.ModeDir | 0755,
					},
					{
						Name:     "one/two/b/file",
						FileMode: 0644,
					},
				},
				windows: []restoreFile{
					{
						Name:     "one",
						FileMode: 0 | os.ModeDir | 0777,
					},
					{
						Name:     "one/two",
						FileMode: 0 | os.ModeDir | 0777,
					},
					{
						Name:     "one/two/three",
						FileMode: 0 | os.ModeDir | 0777,
					},
					{
						Name:     "one/two/three/file-one",
						FileMode: 0666,
					},
					{
						Name:     "one/two/three/file-two",
						FileMode: 0666,
					},
					{
						Name:     "one/two/a",
						FileMode: 0 | os.ModeDir | 0777,
					},
					{
						Name:     "one/two/a/file",
						FileMode: 0666,
					},
					{
						Name:     "one/two/b",
						FileMode: 0 | os.ModeDir | 0777,
					},
					{
						Name:     "one/two/b/file",
						FileMode: 0666,
					},
				},
			},
			wantOutput: wantOutput{
				unix: turbopath.AnchoredUnixPathArray{
					"one",
					"one/two",
					"one/two/a",
					"one/two/b",
					"one/two/three",
					"one/two/a/file",
					"one/two/b/file",
					"one/two/three/file-one",
					"one/two/three/file-two",
				}.ToSystemPathArray(),
			},
		},
		{
			name: "hello world",
			tarFiles: []tarFile{
				{
					Header: &tar.Header{
						Name:     "target",
						Typeflag: tar.TypeReg,
						Mode:     0644,
					},
					Body: "target",
				},
				{
					Header: &tar.Header{
						Name:     "source",
						Linkname: "target",
						Typeflag: tar.TypeSymlink,
						Mode:     0777,
					},
				},
			},
			wantFiles: wantFiles{
				unix: []restoreFile{
					{
						Name:     "source",
						Linkname: "target",
						FileMode: 0 | os.ModeSymlink | 0777,
					},
					{
						Name:     "target",
						FileMode: 0644,
					},
				},
				windows: []restoreFile{
					{
						Name:     "source",
						Linkname: "target",
						FileMode: 0 | os.ModeSymlink | 0666,
					},
					{
						Name:     "target",
						FileMode: 0666,
					},
				},
			},
			wantOutput: wantOutput{
				unix: turbopath.AnchoredUnixPathArray{"target", "source"}.ToSystemPathArray(),
			},
		},
		{
			name: "nested file",
			tarFiles: []tarFile{
				{
					Header: &tar.Header{
						Name:     "folder/",
						Typeflag: tar.TypeDir,
						Mode:     0755,
					},
				},
				{
					Header: &tar.Header{
						Name:     "folder/file",
						Typeflag: tar.TypeReg,
						Mode:     0644,
					},
					Body: "file",
				},
			},
			wantFiles: wantFiles{
				unix: []restoreFile{
					{
						Name:     "folder",
						FileMode: 0 | os.ModeDir | 0755,
					},
					{
						Name:     "folder/file",
						FileMode: 0644,
					},
				},
				windows: []restoreFile{
					{
						Name:     "folder",
						FileMode: 0 | os.ModeDir | 0777,
					},
					{
						Name:     "folder/file",
						FileMode: 0666,
					},
				},
			},
			wantOutput: wantOutput{
				unix: turbopath.AnchoredUnixPathArray{"folder", "folder/file"}.ToSystemPathArray(),
			},
		},
		{
			name: "nested symlink",
			tarFiles: []tarFile{
				{
					Header: &tar.Header{
						Name:     "folder/",
						Typeflag: tar.TypeDir,
						Mode:     0755,
					},
				},
				{
					Header: &tar.Header{
						Name:     "folder/symlink",
						Linkname: "../",
						Typeflag: tar.TypeSymlink,
						Mode:     0777,
					},
				},
				{
					Header: &tar.Header{
						Name:     "folder/symlink/folder-sibling",
						Typeflag: tar.TypeReg,
						Mode:     0644,
					},
					Body: "folder-sibling",
				},
			},
			wantFiles: wantFiles{
				unix: []restoreFile{
					{
						Name:     "folder",
						FileMode: 0 | os.ModeDir | 0755,
					},
					{
						Name:     "folder/symlink",
						FileMode: 0 | os.ModeSymlink | 0777,
						Linkname: "../",
					},
					{
						Name:     "folder/symlink/folder-sibling",
						FileMode: 0644,
					},
					{
						Name:     "folder-sibling",
						FileMode: 0644,
					},
				},
				windows: []restoreFile{
					{
						Name:     "folder",
						FileMode: 0 | os.ModeDir | 0777,
					},
					{
						Name:     "folder/symlink",
						FileMode: 0 | os.ModeSymlink | 0666,
						Linkname: "..\\",
					},
					{
						Name:     "folder/symlink/folder-sibling",
						FileMode: 0666,
					},
					{
						Name:     "folder-sibling",
						FileMode: 0666,
					},
				},
			},
			wantOutput: wantOutput{
				unix: turbopath.AnchoredUnixPathArray{"folder", "folder/symlink", "folder/symlink/folder-sibling"}.ToSystemPathArray(),
			},
		},
		{
			name: "pathological symlinks",
			tarFiles: []tarFile{
				{
					Header: &tar.Header{
						Name:     "one",
						Linkname: "two",
						Typeflag: tar.TypeSymlink,
						Mode:     0777,
					},
				},
				{
					Header: &tar.Header{
						Name:     "two",
						Linkname: "three",
						Typeflag: tar.TypeSymlink,
						Mode:     0777,
					},
				},
				{
					Header: &tar.Header{
						Name:     "three",
						Linkname: "real",
						Typeflag: tar.TypeSymlink,
						Mode:     0777,
					},
				},
				{
					Header: &tar.Header{
						Name:     "real",
						Typeflag: tar.TypeReg,
						Mode:     0755,
					},
					Body: "real",
				},
			},
			wantFiles: wantFiles{
				unix: []restoreFile{
					{
						Name:     "one",
						Linkname: "two",
						FileMode: 0 | os.ModeSymlink | 0777,
					},
					{
						Name:     "two",
						Linkname: "three",
						FileMode: 0 | os.ModeSymlink | 0777,
					},
					{
						Name:     "three",
						Linkname: "real",
						FileMode: 0 | os.ModeSymlink | 0777,
					},
					{
						Name:     "real",
						FileMode: 0 | 0755,
					},
				},
				windows: []restoreFile{
					{
						Name:     "one",
						Linkname: "two",
						FileMode: 0 | os.ModeSymlink | 0666,
					},
					{
						Name:     "two",
						Linkname: "three",
						FileMode: 0 | os.ModeSymlink | 0666,
					},
					{
						Name:     "three",
						Linkname: "real",
						FileMode: 0 | os.ModeSymlink | 0666,
					},
					{
						Name:     "real",
						FileMode: 0 | 0666,
					},
				},
			},
			wantOutput: wantOutput{
				unix: turbopath.AnchoredUnixPathArray{"real", "three", "two", "one"}.ToSystemPathArray(),
			},
		},
		{
			name: "place file at dir location",
			tarFiles: []tarFile{
				{
					Header: &tar.Header{
						Name:     "folder-not-file/",
						Typeflag: tar.TypeDir,
						Mode:     0755,
					},
				},
				{
					Header: &tar.Header{
						Name:     "folder-not-file/subfile",
						Typeflag: tar.TypeReg,
						Mode:     0755,
					},
					Body: "subfile",
				},
				{
					Header: &tar.Header{
						Name:     "folder-not-file",
						Typeflag: tar.TypeReg,
						Mode:     0755,
					},
					Body: "this shouldn't work",
				},
			},
			wantFiles: wantFiles{
				unix: []restoreFile{
					{
						Name:     "folder-not-file",
						FileMode: 0 | os.ModeDir | 0755,
					},
					{
						Name:     "folder-not-file/subfile",
						FileMode: 0755,
					},
				},
				windows: []restoreFile{
					{
						Name:     "folder-not-file",
						FileMode: 0 | os.ModeDir | 0777,
					},
					{
						Name:     "folder-not-file/subfile",
						FileMode: 0666,
					},
				},
			},
			wantOutput: wantOutput{
				unix: turbopath.AnchoredUnixPathArray{"folder-not-file", "folder-not-file/subfile"}.ToSystemPathArray(),
			},
			wantErr: wantErr{
				unix:    syscall.EISDIR,
				windows: syscall.EISDIR,
			},
		},
		// {
		// 	name: "missing symlink with file at subdir",
		// 	tarFiles: []tarFile{
		// 		{
		// 			Header: &tar.Header{
		// 				Name:     "one",
		// 				Linkname: "two",
		// 				Typeflag: tar.TypeSymlink,
		// 				Mode:     0777,
		// 			},
		// 		},
		// 		{
		// 			Header: &tar.Header{
		// 				Name:     "one/file",
		// 				Typeflag: tar.TypeReg,
		// 				Mode:     0755,
		// 			},
		// 			Body: "file",
		// 		},
		// 	},
		// 	wantFiles: wantFiles{
		// 		unix: []restoreFile{
		// 			{
		// 				Name:     "one",
		// 				Linkname: "two",
		// 				FileMode: 0 | os.ModeSymlink | 0777,
		// 			},
		// 		},
		// 	},
		// 	wantOutput: wantOutput{
		// 		unix:    turbopath.AnchoredUnixPathArray{"one"}.ToSystemPathArray(),
		// 		windows: nil,
		// 	},
		// 	wantErr: wantErr{
		// 		unix:    os.ErrExist,
		// 		windows: os.ErrExist,
		// 	},
		// },
		{
			name: "symlink cycle",
			tarFiles: []tarFile{
				{
					Header: &tar.Header{
						Name:     "one",
						Linkname: "two",
						Typeflag: tar.TypeSymlink,
						Mode:     0777,
					},
				},
				{
					Header: &tar.Header{
						Name:     "two",
						Linkname: "three",
						Typeflag: tar.TypeSymlink,
						Mode:     0777,
					},
				},
				{
					Header: &tar.Header{
						Name:     "three",
						Linkname: "one",
						Typeflag: tar.TypeSymlink,
						Mode:     0777,
					},
				},
			},
			wantFiles: wantFiles{
				unix: []restoreFile{},
			},
			wantOutput: wantOutput{
				unix: []turbopath.AnchoredSystemPath{},
			},
			wantErr: wantErr{
				unix:    errCycleDetected,
				windows: errCycleDetected,
			},
		},
		{
			name: "symlink clobber",
			tarFiles: []tarFile{
				{
					Header: &tar.Header{
						Name:     "one",
						Linkname: "two",
						Typeflag: tar.TypeSymlink,
						Mode:     0777,
					},
				},
				{
					Header: &tar.Header{
						Name:     "one",
						Linkname: "three",
						Typeflag: tar.TypeSymlink,
						Mode:     0777,
					},
				},
				{
					Header: &tar.Header{
						Name:     "one",
						Linkname: "real",
						Typeflag: tar.TypeSymlink,
						Mode:     0777,
					},
				},
				{
					Header: &tar.Header{
						Name:     "real",
						Typeflag: tar.TypeReg,
						Mode:     0755,
					},
					Body: "real",
				},
			},
			wantFiles: wantFiles{
				unix: []restoreFile{
					{
						Name:     "one",
						Linkname: "real",
						FileMode: 0 | os.ModeSymlink | 0777,
					},
					{
						Name:     "real",
						FileMode: 0755,
					},
				},
				windows: []restoreFile{
					{
						Name:     "one",
						Linkname: "real",
						FileMode: 0 | os.ModeSymlink | 0666,
					},
					{
						Name:     "real",
						FileMode: 0666,
					},
				},
			},
			wantOutput: wantOutput{
				unix: turbopath.AnchoredUnixPathArray{"real", "one"}.ToSystemPathArray(),
			},
		},
		{
			name: "symlink traversal",
			tarFiles: []tarFile{
				{
					Header: &tar.Header{
						Name:     "escape",
						Linkname: "../",
						Typeflag: tar.TypeSymlink,
						Mode:     0777,
					},
				},
				{
					Header: &tar.Header{
						Name:     "escape/file",
						Typeflag: tar.TypeReg,
						Mode:     0644,
					},
					Body: "file",
				},
			},
			wantFiles: wantFiles{
				unix: []restoreFile{
					{
						Name:     "escape",
						Linkname: "../",
						FileMode: 0 | os.ModeSymlink | 0777,
					},
				},
				windows: []restoreFile{
					{
						Name:     "escape",
						Linkname: "..\\",
						FileMode: 0 | os.ModeSymlink | 0666,
					},
				},
			},
			wantOutput: wantOutput{
				unix: turbopath.AnchoredUnixPathArray{"escape"}.ToSystemPathArray(),
			},
			wantErr: wantErr{
				unix:    errTraversal,
				windows: errTraversal,
			},
		},
		{
			name: "Double indirection: file",
			tarFiles: []tarFile{
				{
					Header: &tar.Header{
						Name:     "up",
						Linkname: "../",
						Typeflag: tar.TypeSymlink,
						Mode:     0777,
					},
				},
				{
					Header: &tar.Header{
						Name:     "link",
						Linkname: "up",
						Typeflag: tar.TypeSymlink,
						Mode:     0777,
					},
				},
				{
					Header: &tar.Header{
						Name:     "link/outside-file",
						Typeflag: tar.TypeReg,
						Mode:     0755,
					},
				},
			},
			wantErr: wantErr{unix: errTraversal, windows: errTraversal},
			wantOutput: wantOutput{
				unix: turbopath.AnchoredUnixPathArray{
					"up",
					"link",
				}.ToSystemPathArray(),
			},
		},
		{
			name: "Double indirection: folder",
			tarFiles: []tarFile{
				{
					Header: &tar.Header{
						Name:     "up",
						Linkname: "../",
						Typeflag: tar.TypeSymlink,
						Mode:     0777,
					},
				},
				{
					Header: &tar.Header{
						Name:     "link",
						Linkname: "up",
						Typeflag: tar.TypeSymlink,
						Mode:     0777,
					},
				},
				{
					Header: &tar.Header{
						Name:     "link/level-one/level-two/",
						Typeflag: tar.TypeDir,
						Mode:     0755,
					},
				},
			},
			wantErr: wantErr{unix: errTraversal, windows: errTraversal},
			wantOutput: wantOutput{
				unix: turbopath.AnchoredUnixPathArray{
					"up",
					"link",
				}.ToSystemPathArray(),
			},
		},
		{
			name: "name traversal",
			tarFiles: []tarFile{
				{
					Header: &tar.Header{
						Name:     "../escape",
						Typeflag: tar.TypeReg,
						Mode:     0644,
					},
					Body: "file",
				},
			},
			wantFiles: wantFiles{
				unix: []restoreFile{},
			},
			wantOutput: wantOutput{
				unix: []turbopath.AnchoredSystemPath{},
			},
			wantErr: wantErr{
				unix:    errNameMalformed,
				windows: errNameMalformed,
			},
		},
		{
			name: "windows unsafe",
			tarFiles: []tarFile{
				{
					Header: &tar.Header{
						Name:     "back\\slash\\file",
						Typeflag: tar.TypeReg,
						Mode:     0644,
					},
					Body: "file",
				},
			},
			wantFiles: wantFiles{
				unix: []restoreFile{
					{
						Name:     "back\\slash\\file",
						FileMode: 0644,
					},
				},
				windows: []restoreFile{},
			},
			wantOutput: wantOutput{
				unix:    turbopath.AnchoredUnixPathArray{"back\\slash\\file"}.ToSystemPathArray(),
				windows: turbopath.AnchoredUnixPathArray{}.ToSystemPathArray(),
			},
			wantErr: wantErr{
				unix:    nil,
				windows: errNameWindowsUnsafe,
			},
		},
		{
			name: "fifo (and others) unsupported",
			tarFiles: []tarFile{
				{
					Header: &tar.Header{
						Name:     "fifo",
						Typeflag: tar.TypeFifo,
					},
				},
			},
			wantFiles: wantFiles{
				unix: []restoreFile{},
			},
			wantOutput: wantOutput{
				unix: []turbopath.AnchoredSystemPath{},
			},
			wantErr: wantErr{
				unix:    errUnsupportedFileType,
				windows: errUnsupportedFileType,
			},
		},
	}
	for _, tt := range tests {
		getTestFunc := func(compressed bool) func(t *testing.T) {
			return func(t *testing.T) {
				var archivePath turbopath.AbsoluteSystemPath
				if compressed {
					archivePath = compressTar(t, generateTar(t, tt.tarFiles))
				} else {
					archivePath = generateTar(t, tt.tarFiles)
				}
				anchor := generateAnchor(t)

				cacheItem, err := Open(archivePath)
				assert.NilError(t, err, "Open")

				restoreOutput, restoreErr := cacheItem.Restore(anchor)
				var desiredErr error
				if runtime.GOOS == "windows" {
					desiredErr = tt.wantErr.windows
				} else {
					desiredErr = tt.wantErr.unix
				}
				if desiredErr != nil {
					if !errors.Is(restoreErr, desiredErr) {
						t.Errorf("wanted err: %v, got err: %v", tt.wantErr, restoreErr)
					}
				} else {
					assert.NilError(t, restoreErr, "Restore")
				}

				outputComparison := tt.wantOutput.unix
				if runtime.GOOS == "windows" && tt.wantOutput.windows != nil {
					outputComparison = tt.wantOutput.windows
				}

				if !reflect.DeepEqual(restoreOutput, outputComparison) {
					t.Errorf("Restore() = %v, want %v", restoreOutput, outputComparison)
				}

				// Check files on disk.
				filesComparison := tt.wantFiles.unix
				if runtime.GOOS == "windows" && tt.wantFiles.windows != nil {
					filesComparison = tt.wantFiles.windows
				}
				for _, diskFile := range filesComparison {
					assertFileExists(t, anchor, diskFile)
				}

				assert.NilError(t, cacheItem.Close(), "Close")
			}
		}
		t.Run(tt.name+"zst", getTestFunc(true))
		t.Run(tt.name, getTestFunc(false))
	}
}

func Test_checkName(t *testing.T) {
	tests := []struct {
		path        string
		wellFormed  bool
		windowsSafe bool
	}{
		// Empty
		{
			path:        "",
			wellFormed:  false,
			windowsSafe: false,
		},
		// Bad prefix
		{
			path:        ".",
			wellFormed:  false,
			windowsSafe: true,
		},
		{
			path:        "..",
			wellFormed:  false,
			windowsSafe: true,
		},
		{
			path:        "/",
			wellFormed:  false,
			windowsSafe: true,
		},
		{
			path:        "./",
			wellFormed:  false,
			windowsSafe: true,
		},
		{
			path:        "../",
			wellFormed:  false,
			windowsSafe: true,
		},
		// Bad prefix, suffixed
		{
			path:        "/a",
			wellFormed:  false,
			windowsSafe: true,
		},
		{
			path:        "./a",
			wellFormed:  false,
			windowsSafe: true,
		},
		{
			path:        "../a",
			wellFormed:  false,
			windowsSafe: true,
		},
		// Bad Suffix
		{
			path:        "/.",
			wellFormed:  false,
			windowsSafe: true,
		},
		{
			path:        "/..",
			wellFormed:  false,
			windowsSafe: true,
		},
		// Bad Suffix, with prefix
		{
			path:        "a/.",
			wellFormed:  false,
			windowsSafe: true,
		},
		{
			path:        "a/..",
			wellFormed:  false,
			windowsSafe: true,
		},
		// Bad middle
		{
			path:        "//",
			wellFormed:  false,
			windowsSafe: true,
		},
		{
			path:        "/./",
			wellFormed:  false,
			windowsSafe: true,
		},
		{
			path:        "/../",
			wellFormed:  false,
			windowsSafe: true,
		},
		// Bad middle, prefixed
		{
			path:        "a//",
			wellFormed:  false,
			windowsSafe: true,
		},
		{
			path:        "a/./",
			wellFormed:  false,
			windowsSafe: true,
		},
		{
			path:        "a/../",
			wellFormed:  false,
			windowsSafe: true,
		},
		// Bad middle, suffixed
		{
			path:        "//a",
			wellFormed:  false,
			windowsSafe: true,
		},
		{
			path:        "/./a",
			wellFormed:  false,
			windowsSafe: true,
		},
		{
			path:        "/../a",
			wellFormed:  false,
			windowsSafe: true,
		},
		// Bad middle, wrapped
		{
			path:        "a//a",
			wellFormed:  false,
			windowsSafe: true,
		},
		{
			path:        "a/./a",
			wellFormed:  false,
			windowsSafe: true,
		},
		{
			path:        "a/../a",
			wellFormed:  false,
			windowsSafe: true,
		},
		// False positive tests
		{
			path:        "...",
			wellFormed:  true,
			windowsSafe: true,
		},
		{
			path:        ".../a",
			wellFormed:  true,
			windowsSafe: true,
		},
		{
			path:        "a/...",
			wellFormed:  true,
			windowsSafe: true,
		},
		{
			path:        "a/.../a",
			wellFormed:  true,
			windowsSafe: true,
		},
		{
			path:        ".../...",
			wellFormed:  true,
			windowsSafe: true,
		},
	}
	for _, tt := range tests {
		t.Run(fmt.Sprintf("Path: \"%v\"", tt.path), func(t *testing.T) {
			wellFormed, windowsSafe := checkName(tt.path)
			if wellFormed != tt.wellFormed || windowsSafe != tt.windowsSafe {
				t.Errorf("\nwantOutput: checkName(\"%v\") wellFormed = %v, windowsSafe %v\ngot:  checkName(\"%v\") wellFormed = %v, windowsSafe %v", tt.path, tt.wellFormed, tt.windowsSafe, tt.path, wellFormed, windowsSafe)
			}
		})
	}
}

func Test_canonicalizeLinkname(t *testing.T) {
	// We're lying that this thing is absolute, but that's not relevant for tests.
	anchor := turbopath.AbsoluteSystemPath(filepath.Join("path", "to", "anchor"))

	tests := []struct {
		name             string
		processedName    turbopath.AnchoredSystemPath
		linkname         string
		canonicalUnix    string
		canonicalWindows string
	}{
		{
			name:             "hello world",
			processedName:    turbopath.AnchoredSystemPath("source"),
			linkname:         "target",
			canonicalUnix:    "path/to/anchor/target",
			canonicalWindows: "path\\to\\anchor\\target",
		},
		{
			name:             "Unix path subdirectory traversal",
			processedName:    turbopath.AnchoredUnixPath("child/source").ToSystemPath(),
			linkname:         "../sibling/target",
			canonicalUnix:    "path/to/anchor/sibling/target",
			canonicalWindows: "path\\to\\anchor\\sibling\\target",
		},
		{
			name:             "Windows path subdirectory traversal",
			processedName:    turbopath.AnchoredUnixPath("child/source").ToSystemPath(),
			linkname:         "..\\sibling\\target",
			canonicalUnix:    "path/to/anchor/child/..\\sibling\\target",
			canonicalWindows: "path\\to\\anchor\\sibling\\target",
		},
	}
	for _, tt := range tests {
		t.Run(tt.name, func(t *testing.T) {
			canonical := tt.canonicalUnix
			if runtime.GOOS == "windows" {
				canonical = tt.canonicalWindows
			}
			if got := canonicalizeLinkname(anchor, tt.processedName, tt.linkname); got != canonical {
				t.Errorf("canonicalizeLinkname() = %v, want %v", got, canonical)
			}
		})
	}
}

func Test_canonicalizeName(t *testing.T) {
	tests := []struct {
		name     string
		fileName string
		want     turbopath.AnchoredSystemPath
		wantErr  error
	}{
		{
			name:     "hello world",
			fileName: "test.txt",
			want:     "test.txt",
		},
		{
			name:     "directory",
			fileName: "something/",
			want:     "something",
		},
		{
			name:     "malformed name",
			fileName: "//",
			want:     "",
			wantErr:  errNameMalformed,
		},
	}
	for _, tt := range tests {
		t.Run(tt.name, func(t *testing.T) {
			got, err := canonicalizeName(tt.fileName)
			if tt.wantErr != nil && !errors.Is(err, tt.wantErr) {
				t.Errorf("canonicalizeName() error = %v, wantErr %v", err, tt.wantErr)
				return
			}
			if !reflect.DeepEqual(got, tt.want) {
				t.Errorf("canonicalizeName() = %v, want %v", got, tt.want)
			}
		})
	}
}

func TestCacheItem_Restore(t *testing.T) {
	tests := []struct {
		name     string
		tarFiles []tarFile
		want     []turbopath.AnchoredSystemPath
	}{
		{
			name: "duplicate restores",
			tarFiles: []tarFile{
				{
					Header: &tar.Header{
						Name:     "target",
						Typeflag: tar.TypeReg,
						Mode:     0644,
					},
					Body: "target",
				},
				{
					Header: &tar.Header{
						Name:     "source",
						Linkname: "target",
						Typeflag: tar.TypeSymlink,
						Mode:     0777,
					},
				},
				{
					Header: &tar.Header{
						Name:     "one/",
						Typeflag: tar.TypeDir,
						Mode:     0755,
					},
				},
				{
					Header: &tar.Header{
						Name:     "one/two/",
						Typeflag: tar.TypeDir,
						Mode:     0755,
					},
				},
			},
			want: turbopath.AnchoredUnixPathArray{"target", "source", "one", "one/two"}.ToSystemPathArray(),
		},
	}
	for _, tt := range tests {
		getTestFunc := func(compressed bool) func(t *testing.T) {
			return func(t *testing.T) {
				var archivePath turbopath.AbsoluteSystemPath
				if compressed {
					archivePath = compressTar(t, generateTar(t, tt.tarFiles))
				} else {
					archivePath = generateTar(t, tt.tarFiles)
				}
				anchor := generateAnchor(t)

				cacheItem, err := Open(archivePath)
				assert.NilError(t, err, "Open")

				restoreOutput, restoreErr := cacheItem.Restore(anchor)
				if !reflect.DeepEqual(restoreOutput, tt.want) {
					t.Errorf("#1 CacheItem.Restore() = %v, want %v", restoreOutput, tt.want)
				}
				assert.NilError(t, restoreErr, "Restore #1")
				assert.NilError(t, cacheItem.Close(), "Close")

				cacheItem2, err2 := Open(archivePath)
				assert.NilError(t, err2, "Open")

				restoreOutput2, restoreErr2 := cacheItem2.Restore(anchor)
				if !reflect.DeepEqual(restoreOutput2, tt.want) {
					t.Errorf("#2 CacheItem.Restore() = %v, want %v", restoreOutput2, tt.want)
				}
				assert.NilError(t, restoreErr2, "Restore #2")
				assert.NilError(t, cacheItem2.Close(), "Close")
			}
		}
		t.Run(tt.name+"zst", getTestFunc(true))
		t.Run(tt.name, getTestFunc(false))
	}
}
