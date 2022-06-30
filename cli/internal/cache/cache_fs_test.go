package cache

import (
	"io/ioutil"
	"os"
	"path/filepath"
	"testing"

	"github.com/vercel/turborepo/cli/internal/analytics"
	"github.com/vercel/turborepo/cli/internal/fs"
	turbofs "github.com/vercel/turborepo/cli/internal/fs"
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
	cwd, err := turbofs.GetCwd()
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

	files := []string{
		filepath.Join(src, filepath.FromSlash("/")),            // src
		filepath.Join(src, filepath.FromSlash("child/")),       // childDir
		filepath.Join(src, filepath.FromSlash("child/a")),      // aPath,
		filepath.Join(src, "b"),                                // bPath,
		filepath.Join(src, filepath.FromSlash("child/link")),   // srcLinkPath,
		filepath.Join(src, filepath.FromSlash("child/broken")), // srcBrokenLinkPath,
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
	got, err := turbofs.SameFile(aPath, dstAPath)
	assert.NilError(t, err, "SameFile")
	if got {
		t.Errorf("SameFile(%v, %v) got true, want false", aPath, dstAPath)
	}

	dstBPath := filepath.Join(dstCachePath, src, "b")
	got, err = turbofs.SameFile(bPath, dstBPath)
	assert.NilError(t, err, "SameFile")
	if got {
		t.Errorf("SameFile(%v, %v) got true, want false", bPath, dstBPath)
	}

	dstLinkPath := filepath.Join(dstCachePath, src, "child", "link")
	target, err := os.Lstat(dstLinkPath)
	assert.NilError(t, err, "Lstat")
	assert.Check(t, target.Mode().IsRegular(), "the cached file is a regular file")

	dstBrokenLinkPath := filepath.Join(dstCachePath, src, "child", "broken")
	_, err = os.Lstat(dstBrokenLinkPath)
	assert.ErrorIs(t, err, os.ErrNotExist)
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
	got, err := turbofs.SameFile(aPath, dstAPath)
	assert.NilError(t, err, "SameFile")
	if got {
		t.Errorf("SameFile(%v, %v) got true, want false", aPath, dstAPath)
	}

	dstBPath := filepath.Join(dstOutputPath, "b")
	got, err = turbofs.SameFile(bPath, dstBPath)
	assert.NilError(t, err, "SameFile")
	if got {
		t.Errorf("SameFile(%v, %v) got true, want false", bPath, dstBPath)
	}

	dstLinkPath := filepath.Join(dstOutputPath, "child", "link")
	dstLstat, dstLstErr := os.Lstat(dstLinkPath)
	assert.NilError(t, dstLstErr, "Lstat")
	assert.Check(t, dstLstat.Mode().IsRegular(), "the cached file is a regular file")

	// We currently don't restore broken symlinks. This is probably a bug
	dstBrokenLinkPath := filepath.Join(dstOutputPath, "child", "broken")
	_, err = os.Readlink(dstBrokenLinkPath)
	assert.ErrorIs(t, err, os.ErrNotExist)
}
