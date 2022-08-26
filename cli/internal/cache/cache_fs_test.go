package cache

import (
	"io/ioutil"
	"os"
	"path/filepath"
	"testing"

	"github.com/vercel/turborepo/cli/internal/analytics"
	"github.com/vercel/turborepo/cli/internal/fs"
	"gotest.tools/v3/assert"
)

type dummyRecorder struct{}

func (dr *dummyRecorder) LogEvent(payload analytics.EventPayload) {}

type testingUtil interface {
	Helper()
	Cleanup(f func())
}

// subdirForTest creates a sub directory of `cwd` and registers it for
// deletion by the testing framework at the end of the test.
// Some cache code currently assumes relative paths from `cwd`, so it is not
// yet feasible to use temp directories.
func subdirForTest(t *testing.T) string {
	var tt interface{} = t
	if tu, ok := tt.(testingUtil); ok {
		tu.Helper()
	}
	cwd, err := fs.GetCwd()
	assert.NilError(t, err, "cwd")
	dir, err := os.MkdirTemp(cwd.ToString(), "turbo-test")
	assert.NilError(t, err, "MkdirTemp")
	deleteOnFinish(t, dir)
	return filepath.Base(dir)
}

func deleteOnFinish(t *testing.T, dir string) {
	var tt interface{} = t
	if tu, ok := tt.(testingUtil); ok {
		tu.Cleanup(func() { _ = os.RemoveAll(dir) })
	}
}

func TestPut(t *testing.T) {
	// Set up a test source and cache directory
	// The "source" directory simulates a package
	//
	// <src>/
	//   b
	//   child/
	//     a
	//     link -> ../b
	//     broken -> missing
	//
	// Ensure we end up with a matching directory under a
	// "cache" directory:
	//
	// <dst>/the-hash/<src>/...

	src := subdirForTest(t)
	childDir := filepath.Join(src, "child")
	err := os.Mkdir(childDir, os.ModeDir|0777)
	assert.NilError(t, err, "Mkdir")
	aPath := filepath.Join(childDir, "a")
	aFile, err := os.Create(aPath)
	assert.NilError(t, err, "Create")
	_, err = aFile.WriteString("hello")
	assert.NilError(t, err, "WriteString")
	assert.NilError(t, aFile.Close(), "Close")

	bPath := filepath.Join(src, "b")
	bFile, err := os.Create(bPath)
	assert.NilError(t, err, "Create")
	_, err = bFile.WriteString("bFile")
	assert.NilError(t, err, "WriteString")
	assert.NilError(t, bFile.Close(), "Close")

	srcLinkPath := filepath.Join(childDir, "link")
	linkTarget := filepath.FromSlash("../b")
	assert.NilError(t, os.Symlink(linkTarget, srcLinkPath), "Symlink")

	srcBrokenLinkPath := filepath.Join(childDir, "broken")
	assert.NilError(t, os.Symlink("missing", srcBrokenLinkPath), "Symlink")
	circlePath := filepath.Join(childDir, "circle")
	assert.NilError(t, os.Symlink(filepath.FromSlash("../child"), circlePath), "Symlink")

	files := []string{
		filepath.Join(src, filepath.FromSlash("/")),            // src
		filepath.Join(src, filepath.FromSlash("child/")),       // childDir
		filepath.Join(src, filepath.FromSlash("child/a")),      // aPath,
		filepath.Join(src, "b"),                                // bPath,
		filepath.Join(src, filepath.FromSlash("child/link")),   // srcLinkPath,
		filepath.Join(src, filepath.FromSlash("child/broken")), // srcBrokenLinkPath,
		filepath.Join(src, filepath.FromSlash("child/circle")), // circlePath
	}

	dst := subdirForTest(t)
	dr := &dummyRecorder{}

	defaultCwd, err := fs.GetCwd()
	if err != nil {
		t.Fatalf("failed to get cwd: %v", err)
	}

	cache := &fsCache{
		cacheDirectory: dst,
		recorder:       dr,
		repoRoot:       defaultCwd,
	}

	hash := "the-hash"
	duration := 0
	err = cache.Put("unused", hash, duration, files)
	assert.NilError(t, err, "Put")

	// Verify that we got the files that we're expecting
	dstCachePath := filepath.Join(dst, hash)

	dstAPath := filepath.Join(dstCachePath, src, "child", "a")
	assertFileMatches(t, aPath, dstAPath)

	dstBPath := filepath.Join(dstCachePath, src, "b")
	assertFileMatches(t, bPath, dstBPath)

	dstLinkPath := filepath.Join(dstCachePath, src, "child", "link")
	target, err := os.Readlink(dstLinkPath)
	assert.NilError(t, err, "Readlink")
	if target != linkTarget {
		t.Errorf("Readlink got %v, want %v", target, linkTarget)
	}

	dstBrokenLinkPath := filepath.Join(dstCachePath, src, "child", "broken")
	target, err = os.Readlink(dstBrokenLinkPath)
	assert.NilError(t, err, "Readlink")
	if target != "missing" {
		t.Errorf("Readlink got %v, want missing", target)
	}

	dstCirclePath := filepath.Join(dstCachePath, src, "child", "circle")
	circleLinkDest, err := os.Readlink(dstCirclePath)
	assert.NilError(t, err, "Readlink")
	expectedCircleLinkDest := filepath.FromSlash("../child")
	if circleLinkDest != expectedCircleLinkDest {
		t.Errorf("Cache link got %v, want %v", circleLinkDest, expectedCircleLinkDest)
	}
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

func TestFetch(t *testing.T) {
	// Set up a test cache directory and target output directory
	// The "cacheDir" directory simulates a cached package
	//
	// <cacheDir>/
	//   the-hash-meta.json
	//   the-hash/
	//     some-package/
	//       b
	//       child/
	//         a
	//         link -> ../b
	//         broken -> missing
	//         circle -> ../child
	//
	// Ensure we end up with a matching directory under a
	// "some-package" directory:
	//
	// "some-package"/...

	cwd, err := fs.GetCwd()
	assert.NilError(t, err, "GetCwd")
	cacheDir := subdirForTest(t)
	src := filepath.Join(cacheDir, "the-hash", "some-package")
	err = os.MkdirAll(src, os.ModeDir|0777)
	assert.NilError(t, err, "mkdirAll")

	childDir := filepath.Join(src, "child")
	err = os.Mkdir(childDir, os.ModeDir|0777)
	assert.NilError(t, err, "Mkdir")
	aPath := filepath.Join(childDir, "a")
	aFile, err := os.Create(aPath)
	assert.NilError(t, err, "Create")
	_, err = aFile.WriteString("hello")
	assert.NilError(t, err, "WriteString")
	assert.NilError(t, aFile.Close(), "Close")

	bPath := filepath.Join(src, "b")
	bFile, err := os.Create(bPath)
	assert.NilError(t, err, "Create")
	_, err = bFile.WriteString("bFile")
	assert.NilError(t, err, "WriteString")
	assert.NilError(t, bFile.Close(), "Close")

	srcLinkPath := filepath.Join(childDir, "link")
	linkTarget := filepath.FromSlash("../b")
	assert.NilError(t, os.Symlink(linkTarget, srcLinkPath), "Symlink")

	srcBrokenLinkPath := filepath.Join(childDir, "broken")
	assert.NilError(t, os.Symlink("missing", srcBrokenLinkPath), "Symlink")
	circlePath := filepath.Join(childDir, "circle")
	assert.NilError(t, os.Symlink(filepath.FromSlash("../child"), circlePath), "Symlink")

	metadataPath := filepath.Join(cacheDir, "the-hash-meta.json")
	err = ioutil.WriteFile(metadataPath, []byte(`{"hash":"the-hash","duration":0}`), 0777)
	assert.NilError(t, err, "WriteFile")

	dr := &dummyRecorder{}

	defaultCwd, err := fs.GetCwd()
	if err != nil {
		t.Fatalf("failed to get cwd: %v", err)
	}

	cache := &fsCache{
		cacheDirectory: cacheDir,
		recorder:       dr,
		repoRoot:       defaultCwd,
	}

	dstOutputPath := "some-package"
	deleteOnFinish(t, dstOutputPath)
	hit, files, _, err := cache.Fetch(cwd.ToStringDuringMigration(), "the-hash", []string{})
	assert.NilError(t, err, "Fetch")
	if !hit {
		t.Error("Fetch got false, want true")
	}
	if len(files) != 0 {
		// Not for any particular reason, but currently the fs cache doesn't return the
		// list of files copied
		t.Errorf("len(files) got %v, want 0", len(files))
	}
	t.Logf("files %v", files)

	dstAPath := filepath.Join(dstOutputPath, "child", "a")
	assertFileMatches(t, aPath, dstAPath)

	dstBPath := filepath.Join(dstOutputPath, "b")
	assertFileMatches(t, bPath, dstBPath)

	dstLinkPath := filepath.Join(dstOutputPath, "child", "link")
	target, err := os.Readlink(dstLinkPath)
	assert.NilError(t, err, "Readlink")
	if target != linkTarget {
		t.Errorf("Readlink got %v, want %v", target, linkTarget)
	}

	// We currently don't restore broken symlinks. This is probably a bug
	dstBrokenLinkPath := filepath.Join(dstOutputPath, "child", "broken")
	_, err = os.Readlink(dstBrokenLinkPath)
	assert.ErrorIs(t, err, os.ErrNotExist)

	// Currently, on restore, we convert symlink-to-directory to empty-directory
	// This is very likely not ideal behavior, but leaving this test here to verify
	// that it is what we expect at this point in time.
	dstCirclePath := filepath.Join(dstOutputPath, "child", "circle")
	circleStat, err := os.Lstat(dstCirclePath)
	assert.NilError(t, err, "Lstat")
	assert.Equal(t, circleStat.IsDir(), true)
	entries, err := os.ReadDir(dstCirclePath)
	assert.NilError(t, err, "ReadDir")
	assert.Equal(t, len(entries), 0)
}
