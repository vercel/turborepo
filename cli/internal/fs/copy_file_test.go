package fs

import (
	"errors"
	"os"
	"testing"

	"github.com/vercel/turbo/cli/internal/turbopath"
	"gotest.tools/v3/assert"
)

func TestCopyFile(t *testing.T) {
	srcTmpDir := turbopath.AbsoluteSystemPath(t.TempDir())
	destTmpDir := turbopath.AbsoluteSystemPath(t.TempDir())
	srcFilePath := srcTmpDir.UntypedJoin("src")
	destFilePath := destTmpDir.UntypedJoin("dest")
	from := &LstatCachedFile{Path: srcFilePath}

	// The src file doesn't exist, will error.
	err := CopyFile(from, destFilePath.ToString())
	pathErr := &os.PathError{}
	if !errors.As(err, &pathErr) {
		t.Errorf("got %v, want PathError", err)
	}

	// Create the src file.
	srcFile, err := srcFilePath.Create()
	assert.NilError(t, err, "Create")
	_, err = srcFile.WriteString("src")
	assert.NilError(t, err, "WriteString")
	assert.NilError(t, srcFile.Close(), "Close")

	// Copy the src to the dest.
	err = CopyFile(from, destFilePath.ToString())
	assert.NilError(t, err, "src exists dest does not, should not error.")

	// Now test for symlinks.
	symlinkSrcDir := turbopath.AbsoluteSystemPath(t.TempDir())
	symlinkTargetDir := turbopath.AbsoluteSystemPath(t.TempDir())
	symlinkDestDir := turbopath.AbsoluteSystemPath(t.TempDir())
	symlinkSrcPath := symlinkSrcDir.UntypedJoin("symlink")
	symlinkTargetPath := symlinkTargetDir.UntypedJoin("target")
	symlinkDestPath := symlinkDestDir.UntypedJoin("dest")
	fromSymlink := &LstatCachedFile{Path: symlinkSrcPath}

	// Create the symlink target.
	symlinkTargetFile, err := symlinkTargetPath.Create()
	assert.NilError(t, err, "Create")
	_, err = symlinkTargetFile.WriteString("Target")
	assert.NilError(t, err, "WriteString")
	assert.NilError(t, symlinkTargetFile.Close(), "Close")

	// Link things up.
	err = symlinkSrcPath.Symlink(symlinkTargetPath.ToString())
	assert.NilError(t, err, "Symlink")

	// Run the test.
	err = CopyFile(fromSymlink, symlinkDestPath.ToString())
	assert.NilError(t, err, "Copying a valid symlink does not error.")

	// Break the symlink.
	err = symlinkTargetPath.Remove()
	assert.NilError(t, err, "breaking the symlink")

	// Remove the existing copy.
	err = symlinkDestPath.Remove()
	assert.NilError(t, err, "existing copy is removed")

	// Try copying the now-broken symlink.
	err = CopyFile(fromSymlink, symlinkDestPath.ToString())
	assert.NilError(t, err, "CopyFile")

	// Confirm that it copied
	target, err := symlinkDestPath.Readlink()
	assert.NilError(t, err, "Readlink")
	assert.Equal(t, target, symlinkTargetPath.ToString())
}

func TestCopyOrLinkFileWithPerms(t *testing.T) {
	// Directory layout:
	//
	// <src>/
	//   foo
	readonlyMode := os.FileMode(0444)
	srcDir := turbopath.AbsoluteSystemPath(t.TempDir())
	dstDir := turbopath.AbsoluteSystemPath(t.TempDir())
	srcFilePath := srcDir.UntypedJoin("src")
	dstFilePath := dstDir.UntypedJoin("dst")
	srcFile, err := srcFilePath.Create()
	defer func() { _ = srcFile.Close() }()
	assert.NilError(t, err, "Create")
	err = srcFile.Chmod(readonlyMode)
	assert.NilError(t, err, "Chmod")
	err = CopyFile(&LstatCachedFile{Path: srcFilePath}, dstFilePath.ToStringDuringMigration())
	assert.NilError(t, err, "CopyOrLinkFile")
	info, err := dstFilePath.Lstat()
	assert.NilError(t, err, "Lstat")
	assert.Equal(t, info.Mode(), readonlyMode, "expected dest to have matching permissions")
}
