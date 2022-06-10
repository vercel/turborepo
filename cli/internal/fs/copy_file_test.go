package fs

import (
	"os"
	"path/filepath"
	"testing"

	"gotest.tools/v3/assert"
	"gotest.tools/v3/fs"
)

func TestCopyFile(t *testing.T) {
	// Directory layout:
	//
	// <src>/
	//   foo
	src := fs.NewDir(t, "copy")
	dst := fs.NewDir(t, "copy-dist")
	srcFilePath := filepath.Join(src.Path(), "foo")
	dstFilePath := filepath.Join(dst.Path(), "foo")
	srcFile, err := os.Create(srcFilePath)
	assert.NilError(t, err, "Create")
	stat, err := srcFile.Stat()
	assert.NilError(t, err, "Stat")
	err = CopyFile(srcFilePath, dstFilePath, stat.Mode())
	assert.NilError(t, err, "CopyFile")

	// Directory layout:
	//
	// <src>/
	//   foo
	//   foo-ptr -> foo
	srcLinkPath := filepath.Join(src.Path(), "foo-ptr")
	dstLinkPath := filepath.Join(dst.Path(), "foo-ptr")
	err = os.Symlink("foo", srcLinkPath)
	assert.NilError(t, err, "SymLink")
	stat, err = os.Lstat(srcLinkPath)
	assert.NilError(t, err, "Lstat")
	err = CopyFile(srcLinkPath, dstLinkPath, stat.Mode())
	if err != nil {
		t.Fatalf("CopyFile %v", err)
	}
	linkDst, err := os.Stat(dstLinkPath)
	assert.NilError(t, err, "Stat")
	assert.Check(t, linkDst.Mode().IsRegular(), "the target is a regular file")
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
	src := fs.NewDir(t, "recursive-copy")
	dst := fs.NewDir(t, "recursive-copy-dist")
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

	mode := os.ModeDir // TODO(gsoltis): this mode argument seems out of place
	err = RecursiveCopy(src.Path(), dst.Path(), mode)
	assert.NilError(t, err, "RecursiveCopy")

	dstLinkPath := filepath.Join(dst.Path(), "child", "link")
	dstLinkDest, err := os.Stat(dstLinkPath)
	assert.NilError(t, err, "Stat")
	assert.Check(t, dstLinkDest.Mode().IsRegular(), "the target is a regular file")
	dstBrokenLinkPath := filepath.Join(dst.Path(), "child", "broken")
	brokenLinkExists := PathExists(dstBrokenLinkPath)
	if brokenLinkExists {
		t.Errorf("We cached a broken link at %v", dstBrokenLinkPath)
	}
}
