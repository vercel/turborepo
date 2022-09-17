package cacheitem

import (
	"archive/tar"
	"compress/gzip"
	"fmt"
	"os"
	"path/filepath"
	"reflect"
	"runtime"
	"testing"

	"github.com/vercel/turborepo/cli/internal/turbopath"
	"gotest.tools/v3/assert"
)

type tarFiles []struct {
	Body string
	*tar.Header
}

// This function is used specifically to generate tar files that Turborepo would
// rarely or never encounter without malicious or pathological inputs. We use it
// to make sure that we respond well in these scenarios during restore attempts.
func generateTar(t *testing.T, files tarFiles) turbopath.AbsoluteSystemPath {
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

func generateRestorePoint(t *testing.T) turbopath.AbsoluteSystemPath {
	t.Helper()
	testDir := t.TempDir()
	anchorPoint := filepath.Join(testDir, "anchor")

	mkdirErr := os.Mkdir(anchorPoint, 0777)
	assert.NilError(t, mkdirErr, "Mkdir")

	return turbopath.AbsoluteSystemPath(anchorPoint)
}

func TestOpen(t *testing.T) {
	tests := []struct {
		name     string
		tarFiles tarFiles
		want     []turbopath.AnchoredSystemPath
		wantErr  bool
	}{
		{
			name: "hello world",
			tarFiles: tarFiles{
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
			want: []turbopath.AnchoredSystemPath{"target", "source"},
		},
	}
	for _, tt := range tests {
		t.Run(tt.name, func(t *testing.T) {
			archivePath := generateTar(t, tt.tarFiles)
			restorePoint := generateRestorePoint(t)

			archive, err := Open(archivePath)
			assert.NilError(t, err, "Open")
			defer func() { _ = archive.Close() }()

			restoreOutput, restoreErr := archive.Restore(restorePoint)
			assert.NilError(t, restoreErr, "Restore")

			if !reflect.DeepEqual(restoreOutput, tt.want) {
				t.Errorf("Restore() = %v, want %v", restoreOutput, tt.want)
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
