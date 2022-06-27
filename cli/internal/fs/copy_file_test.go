package fs

import (
	"os"
	"path/filepath"
	"testing"

	"gotest.tools/v3/assert"
	"gotest.tools/v3/fs"
)

func TestCopyOrLinkFile(t *testing.T) {
	// Directory layout:
	//
	// <src>/
	//   foo
	src := fs.NewDir(t, "copy-or-link")
	dst := fs.NewDir(t, "copy-or-link-dist")
	srcFilePath := filepath.Join(src.Path(), "foo")
	dstFilePath := filepath.Join(dst.Path(), "foo")
	_, err := os.Create(srcFilePath)
	assert.NilError(t, err, "Create")
	assert.NilError(t, err, "Stat")
	shouldLink := true
	shouldFallback := false
	err = CopyOrLinkFile(&LstatCachedFile{Path: AbsolutePath(srcFilePath)}, dstFilePath, shouldLink, shouldFallback)
	assert.NilError(t, err, "CopyOrLinkFile")
	sameFile, err := SameFile(srcFilePath, dstFilePath)
	assert.NilError(t, err, "SameFile")
	if !sameFile {
		t.Errorf("SameFile(%v, %v) got false, want true", srcFilePath, dstFilePath)
	}

	// Directory layout:
	//
	// <src>/
	//   foo
	//   foo-ptr -> foo
	srcLinkPath := filepath.Join(src.Path(), "foo-ptr")
	dstLinkPath := filepath.Join(dst.Path(), "foo-ptr")
	err = os.Symlink("foo", srcLinkPath)
	assert.NilError(t, err, "SymLink")
	assert.NilError(t, err, "Lstat")
	err = CopyOrLinkFile(&LstatCachedFile{Path: AbsolutePath(srcLinkPath)}, dstLinkPath, shouldLink, shouldFallback)
	if err != nil {
		t.Fatalf("CopyOrLinkFile %v", err)
	}
	linkDst, err := os.Readlink(dstLinkPath)
	assert.NilError(t, err, "Readlink")
	if linkDst != "foo" {
		t.Errorf("Readlink(dstLinkPath) got %v, want foo", linkDst)
	}
}

func TestCopyOrLinkFileWithPerms(t *testing.T) {
	// Directory layout:
	//
	// <src>/
	//   foo
	readonlyMode := os.FileMode(0444)
	src := fs.NewDir(t, "copy-or-link")
	dst := fs.NewDir(t, "copy-or-link-dist")
	srcFilePath := filepath.Join(src.Path(), "foo")
	dstFilePath := filepath.Join(dst.Path(), "foo")
	srcFile, err := os.Create(srcFilePath)
	assert.NilError(t, err, "Create")
	err = srcFile.Chmod(readonlyMode)
	assert.NilError(t, err, "Chmod")
	shouldLink := false
	shouldFallback := false
	err = CopyOrLinkFile(&LstatCachedFile{Path: AbsolutePath(srcFilePath)}, dstFilePath, shouldLink, shouldFallback)
	assert.NilError(t, err, "CopyOrLinkFile")
	sameFile, err := SameFile(srcFilePath, dstFilePath)
	assert.NilError(t, err, "SameFile")
	if sameFile {
		t.Errorf("SameFile(%v, %v) got true, want false", srcFilePath, dstFilePath)
	}
	info, err := os.Lstat(dstFilePath)
	assert.NilError(t, err, "Lstat")
	assert.Equal(t, info.Mode(), readonlyMode, "expected dest to have matching permissions")
}

func TestRecursiveCopyOrLinkFile(t *testing.T) {
	// Directory layout:
	//
	// <src>/
	//   b
	//   child/
	//     a
	//     link -> ../b
	//     broken -> missing
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

	shouldLink := true
	shouldFallback := false
	err = RecursiveCopyOrLinkFile(src.Path(), dst.Path(), shouldLink, shouldFallback)
	assert.NilError(t, err, "RecursiveCopyOrLinkFile")

	dstAPath := filepath.Join(dst.Path(), "child", "a")
	got, err := SameFile(aPath, dstAPath)
	assert.NilError(t, err, "SameFile")
	if !got {
		t.Errorf("SameFile(%v, %v) got false, want true", aPath, dstAPath)
	}
	dstBPath := filepath.Join(dst.Path(), "b")
	got, err = SameFile(bPath, dstBPath)
	assert.NilError(t, err, "SameFile")
	if !got {
		t.Errorf("SameFile(%v, %v) got false, want true", bPath, dstBPath)
	}
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
}

func TestSameFile(t *testing.T) {
	a := fs.NewFile(t, "a")
	b := filepath.Join(filepath.Dir(a.Path()), "b")
	err := os.Link(a.Path(), b)
	defer func() { _ = os.Remove(b) }()
	if err != nil {
		t.Fatalf("failed linking %v", err)
	}
	got, err := SameFile(a.Path(), b)
	if err != nil {
		t.Fatalf("failed to check if a is the same file as b: %v", err)
	}
	if !got {
		t.Error("SameFile got false, want true")
	}

	got, err = SameFile(b, b)
	if err != nil {
		t.Fatalf("failed to check if b is the same file as b: %v", err)
	}
	if !got {
		t.Error("SameFile got false, want true")
	}

	c := fs.NewFile(t, "c")
	got, err = SameFile(b, c.Path())
	if err != nil {
		t.Fatalf("failed to check if b is the same file as c: %v", c)
	}
	if got {
		t.Error("SameFile got true, want false")
	}
}
