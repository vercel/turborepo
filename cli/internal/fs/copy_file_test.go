package fs

import (
	"errors"
	"io/ioutil"
	"os"
	"path/filepath"
	"testing"

	"github.com/vercel/turbo/cli/internal/turbopath"
	"gotest.tools/v3/assert"
	"gotest.tools/v3/fs"
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

func TestRecursiveCopy(t *testing.T) {
	// Directory layout:
	//
	// <src>/
	//   b
	//   child/
	//     a
	//     link -> ../b
	//     broken -> missing
	//     circle -> ../child
	src := fs.NewDir(t, "recursive-copy-or-link")
	dst := fs.NewDir(t, "recursive-copy-or-link-dist")
	childDir := filepath.Join(src.Path(), "child")
	err := os.Mkdir(childDir, os.ModeDir|0777)
	assert.NilError(t, err, "Mkdir")
	aPath := filepath.Join(childDir, "a")
	aFile, err := os.Create(aPath)
	assert.NilError(t, err, "Create")
	_, err = aFile.WriteString("hello")
	assert.NilError(t, err, "WriteString")
	assert.NilError(t, aFile.Close(), "Close")

	bPath := filepath.Join(src.Path(), "b")
	bFile, err := os.Create(bPath)
	assert.NilError(t, err, "Create")
	_, err = bFile.WriteString("bFile")
	assert.NilError(t, err, "WriteString")
	assert.NilError(t, bFile.Close(), "Close")

	srcLinkPath := filepath.Join(childDir, "link")
	assert.NilError(t, os.Symlink(filepath.FromSlash("../b"), srcLinkPath), "Symlink")

	srcBrokenLinkPath := filepath.Join(childDir, "broken")
	assert.NilError(t, os.Symlink("missing", srcBrokenLinkPath), "Symlink")
	circlePath := filepath.Join(childDir, "circle")
	assert.NilError(t, os.Symlink(filepath.FromSlash("../child"), circlePath), "Symlink")

	err = RecursiveCopy(turbopath.AbsoluteSystemPathFromUpstream(src.Path()), turbopath.AbsoluteSystemPathFromUpstream(dst.Path()))
	assert.NilError(t, err, "RecursiveCopy")
	// For ensure multiple times copy will not broken
	err = RecursiveCopy(turbopath.AbsoluteSystemPathFromUpstream(src.Path()), turbopath.AbsoluteSystemPathFromUpstream(dst.Path()))
	assert.NilError(t, err, "RecursiveCopy")

	dstChildDir := filepath.Join(dst.Path(), "child")
	assertDirMatches(t, childDir, dstChildDir)
	dstAPath := filepath.Join(dst.Path(), "child", "a")
	assertFileMatches(t, aPath, dstAPath)
	dstBPath := filepath.Join(dst.Path(), "b")
	assertFileMatches(t, bPath, dstBPath)
	dstLinkPath := filepath.Join(dst.Path(), "child", "link")
	dstLinkDest, err := os.Readlink(dstLinkPath)
	assert.NilError(t, err, "Readlink")
	expectedLinkDest := filepath.FromSlash("../b")
	if dstLinkDest != expectedLinkDest {
		t.Errorf("Readlink got %v, want %v", dstLinkDest, expectedLinkDest)
	}
	dstBrokenLinkPath := filepath.Join(dst.Path(), "child", "broken")
	brokenLinkExists := PathExists(dstBrokenLinkPath)
	if brokenLinkExists {
		t.Errorf("We cached a broken link at %v", dstBrokenLinkPath)
	}
	// Currently, we convert symlink-to-directory to empty-directory
	// This is very likely not ideal behavior, but leaving this test here to verify
	// that it is what we expect at this point in time.
	dstCirclePath := filepath.Join(dst.Path(), "child", "circle")
	circleStat, err := os.Lstat(dstCirclePath)
	assert.NilError(t, err, "Lstat")
	assert.Equal(t, circleStat.IsDir(), true)
	entries, err := os.ReadDir(dstCirclePath)
	assert.NilError(t, err, "ReadDir")
	assert.Equal(t, len(entries), 0)
}

func assertFileMatches(t *testing.T, orig string, copy string) {
	t.Helper()
	origBytes, err := ioutil.ReadFile(orig)
	assert.NilError(t, err, "ReadFile")
	copyBytes, err := ioutil.ReadFile(copy)
	assert.NilError(t, err, "ReadFile")
	assert.DeepEqual(t, origBytes, copyBytes)
	origStat, err := os.Lstat(orig)
	assert.NilError(t, err, "Lstat")
	copyStat, err := os.Lstat(copy)
	assert.NilError(t, err, "Lstat")
	assert.Equal(t, origStat.Mode(), copyStat.Mode())
}

func assertDirMatches(t *testing.T, orig string, copy string) {
	t.Helper()
	origStat, err := os.Lstat(orig)
	assert.NilError(t, err, "Lstat")
	copyStat, err := os.Lstat(copy)
	assert.NilError(t, err, "Lstat")
	assert.Equal(t, origStat.Mode(), copyStat.Mode())
}
