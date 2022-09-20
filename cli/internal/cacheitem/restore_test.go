package cacheitem

import (
	"archive/tar"
	"compress/gzip"
	"errors"
	"fmt"
	"io/fs"
	"os"
	"path/filepath"
	"reflect"
	"runtime"
	"syscall"
	"testing"

	"github.com/vercel/turborepo/cli/internal/turbopath"
	"gotest.tools/v3/assert"
)

type tarFile struct {
	Body string
	*tar.Header
}

type diskFile struct {
	Name     string
	Linkname string
	fs.FileMode
}

// This function is used specifically to generate tar files that Turborepo would
// rarely or never encounter without malicious or pathological inputs. We use it
// to make sure that we respond well in these scenarios during restore attempts.
func generateTar(t *testing.T, files []tarFile) turbopath.AbsoluteSystemPath {
	t.Helper()
	testDir := t.TempDir()
	testArchivePath := filepath.Join(testDir, "out.tar.gz")

	handle, handleCreateErr := os.Create(testArchivePath)
	assert.NilError(t, handleCreateErr, "os.Create")

	gzw := gzip.NewWriter(handle)
	tw := tar.NewWriter(gzw)

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

	gzwCloseErr := gzw.Close()
	assert.NilError(t, gzwCloseErr, "gzw.Close")

	handleCloseErr := handle.Close()
	assert.NilError(t, handleCloseErr, "handle.Close")

	return turbopath.AbsoluteSystemPath(testArchivePath)
}

func generateAnchor(t *testing.T) turbopath.AbsoluteSystemPath {
	t.Helper()
	testDir := t.TempDir()
	anchorPoint := filepath.Join(testDir, "anchor")

	mkdirErr := os.Mkdir(anchorPoint, 0777)
	assert.NilError(t, mkdirErr, "Mkdir")

	return turbopath.AbsoluteSystemPath(anchorPoint)
}

func assertFileExists(t *testing.T, anchor turbopath.AbsoluteSystemPath, diskFile diskFile) {
	t.Helper()
	// If we have gotten here we can assume this to be true.
	processedName := turbopath.AnchoredSystemPath(diskFile.Name)
	fullName := processedName.RestoreAnchor(anchor)
	fileInfo, err := os.Lstat(fullName.ToString())
	assert.NilError(t, err, "Lstat")

	assert.Equal(t, fileInfo.Mode()&diskFile.FileMode, diskFile.FileMode, "File has the expected mode.")

	if diskFile.FileMode&os.ModeSymlink != 0 {
		linkname, err := os.Readlink(fullName.ToString())
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
	tests := []struct {
		name      string
		tarFiles  []tarFile
		want      []turbopath.AnchoredSystemPath
		wantFiles []diskFile
		wantErr   wantErr
	}{
		{
			name: "hello world",
			tarFiles: []tarFile{
				{
					Header: &tar.Header{
						Name:     "target",
						Typeflag: tar.TypeReg,
					},
					Body: "target",
				},
				{
					Header: &tar.Header{
						Name:     "source",
						Linkname: "target",
						Typeflag: tar.TypeSymlink,
					},
				},
			},
			wantFiles: []diskFile{
				{
					Name:     "source",
					Linkname: "target",
					FileMode: 0 | os.ModeSymlink,
				},
				{
					Name:     "target",
					FileMode: 0,
				},
			},
			want: []turbopath.AnchoredSystemPath{"target", "source"},
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
					},
					Body: "file",
				},
			},
			wantFiles: []diskFile{
				{
					Name:     "folder",
					FileMode: 0 | os.ModeDir | 0755,
				},
				{
					Name:     "folder/file",
					FileMode: 0,
				},
			},
			want: []turbopath.AnchoredSystemPath{"folder/", "folder/file"},
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
						Name:     "folder/symlink",
						Linkname: "../",
						Typeflag: tar.TypeSymlink,
					},
				},
				{
					Header: &tar.Header{
						Name:     "folder/symlink/folder-sibling",
						Typeflag: tar.TypeReg,
					},
					Body: "folder-sibling",
				},
			},
			wantFiles: []diskFile{
				{
					Name:     "folder",
					FileMode: 0 | os.ModeDir | 0755,
				},
				{
					Name:     "folder/symlink",
					FileMode: 0 | os.ModeSymlink,
					Linkname: "../",
				},
				{
					Name:     "folder/symlink/folder-sibling",
					FileMode: 0,
				},
				{
					Name:     "folder-sibling",
					FileMode: 0,
				},
			},
			want: []turbopath.AnchoredSystemPath{"folder/", "folder/symlink", "folder/symlink/folder-sibling"},
		},
		{
			name: "pathological symlinks",
			tarFiles: []tarFile{
				{
					Header: &tar.Header{
						Name:     "one",
						Linkname: "two",
						Typeflag: tar.TypeSymlink,
					},
				},
				{
					Header: &tar.Header{
						Name:     "two",
						Linkname: "three",
						Typeflag: tar.TypeSymlink,
					},
				},
				{
					Header: &tar.Header{
						Name:     "three",
						Linkname: "real",
						Typeflag: tar.TypeSymlink,
					},
				},
				{
					Header: &tar.Header{
						Name:     "real",
						Typeflag: tar.TypeReg,
					},
					Body: "real",
				},
			},
			wantFiles: []diskFile{
				{
					Name:     "one",
					Linkname: "two",
					FileMode: 0 | os.ModeSymlink,
				},
				{
					Name:     "two",
					Linkname: "three",
					FileMode: 0 | os.ModeSymlink,
				},
				{
					Name:     "three",
					Linkname: "real",
					FileMode: 0 | os.ModeSymlink,
				},
				{
					Name:     "real",
					FileMode: 0,
				},
			},
			want: []turbopath.AnchoredSystemPath{"real", "three", "two", "one"},
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
			wantFiles: []diskFile{
				{
					Name:     "folder-not-file",
					FileMode: 0 | os.ModeDir,
				},
				{
					Name:     "folder-not-file/subfile",
					FileMode: 0,
				},
			},
			want: []turbopath.AnchoredSystemPath{"folder-not-file/", "folder-not-file/subfile"},
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
		// 				Mode:     0755,
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
		// 	wantFiles: []diskFile{
		// 		{
		// 			Name:     "one",
		// 			Linkname: "two",
		// 			FileMode: 0 | os.ModeSymlink,
		// 		},
		// 	},
		// 	want: []turbopath.AnchoredSystemPath{"one"},
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
					},
				},
				{
					Header: &tar.Header{
						Name:     "two",
						Linkname: "three",
						Typeflag: tar.TypeSymlink,
					},
				},
				{
					Header: &tar.Header{
						Name:     "three",
						Linkname: "one",
						Typeflag: tar.TypeSymlink,
					},
				},
			},
			wantFiles: []diskFile{},
			want:      []turbopath.AnchoredSystemPath{},
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
					},
				},
				{
					Header: &tar.Header{
						Name:     "one",
						Linkname: "three",
						Typeflag: tar.TypeSymlink,
					},
				},
				{
					Header: &tar.Header{
						Name:     "one",
						Linkname: "real",
						Typeflag: tar.TypeSymlink,
					},
				},
				{
					Header: &tar.Header{
						Name:     "real",
						Typeflag: tar.TypeReg,
					},
					Body: "real",
				},
			},
			wantFiles: []diskFile{
				{
					Name:     "one",
					Linkname: "real",
					FileMode: 0 | os.ModeSymlink,
				},
				{
					Name:     "real",
					FileMode: 0,
				},
			},
			want: []turbopath.AnchoredSystemPath{"real", "one"},
		},
		{
			name: "symlink traversal",
			tarFiles: []tarFile{
				{
					Header: &tar.Header{
						Name:     "escape",
						Linkname: "../",
						Typeflag: tar.TypeSymlink,
					},
				},
				{
					Header: &tar.Header{
						Name:     "escape/file",
						Typeflag: tar.TypeReg,
					},
					Body: "file",
				},
			},
			wantFiles: []diskFile{
				{
					Name:     "escape",
					Linkname: "../",
					FileMode: 0 | os.ModeSymlink,
				},
			},
			want: []turbopath.AnchoredSystemPath{"escape"},
			wantErr: wantErr{
				unix:    errTraversal,
				windows: errTraversal,
			},
		},
		{
			name: "name traversal",
			tarFiles: []tarFile{
				{
					Header: &tar.Header{
						Name:     "../escape",
						Typeflag: tar.TypeReg,
					},
					Body: "file",
				},
			},
			wantFiles: []diskFile{},
			want:      []turbopath.AnchoredSystemPath{},
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
					},
					Body: "file",
				},
			},
			wantFiles: []diskFile{
				{
					Name:     "back\\slash\\file",
					FileMode: 0,
				},
			},
			want: []turbopath.AnchoredSystemPath{"back\\slash\\file"},
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
			wantFiles: []diskFile{},
			want:      []turbopath.AnchoredSystemPath{},
			wantErr: wantErr{
				unix:    errUnsupportedFileType,
				windows: errUnsupportedFileType,
			},
		},
	}
	for _, tt := range tests {
		t.Run(tt.name, func(t *testing.T) {
			archivePath := generateTar(t, tt.tarFiles)
			anchor := generateAnchor(t)

			archive, err := Open(archivePath)
			assert.NilError(t, err, "Open")
			defer func() { _ = archive.Close() }()

			restoreOutput, restoreErr := archive.Restore(anchor)
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

			if !reflect.DeepEqual(restoreOutput, tt.want) {
				t.Errorf("Restore() = %v, want %v", restoreOutput, tt.want)
			}

			// Check files on disk.
			for _, diskFile := range tt.wantFiles {
				assertFileExists(t, anchor, diskFile)
			}
		})
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
				t.Errorf("\nwant: checkName(\"%v\") wellFormed = %v, windowsSafe %v\ngot:  checkName(\"%v\") wellFormed = %v, windowsSafe %v", tt.path, tt.wellFormed, tt.windowsSafe, tt.path, wellFormed, windowsSafe)
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
			processedName:    turbopath.AnchoredSystemPath(filepath.Join("child", "source")),
			linkname:         "../sibling/target",
			canonicalUnix:    "path/to/anchor/sibling/target",
			canonicalWindows: "path\\to\\anchor\\sibling\\target",
		},
		{
			name:             "Windows path subdirectory traversal",
			processedName:    turbopath.AnchoredSystemPath(filepath.Join("child", "source")),
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
