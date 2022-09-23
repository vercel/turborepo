package cache

import (
	"io/ioutil"
	"os"
	"path/filepath"
	"testing"

	"github.com/vercel/turborepo/cli/internal/analytics"
	"github.com/vercel/turborepo/cli/internal/fs"
	"github.com/vercel/turborepo/cli/internal/turbopath"
	"gotest.tools/v3/assert"
)

type dummyRecorder struct{}

func (dr *dummyRecorder) LogEvent(payload analytics.EventPayload) {}

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

	src := turbopath.AbsoluteSystemPath(t.TempDir())
	childDir := src.UntypedJoin("child")
	err := childDir.MkdirAll()
	assert.NilError(t, err, "Mkdir")
	aPath := childDir.UntypedJoin("a")
	aFile, err := aPath.Create()
	assert.NilError(t, err, "Create")
	_, err = aFile.WriteString("hello")
	assert.NilError(t, err, "WriteString")
	assert.NilError(t, aFile.Close(), "Close")

	bPath := src.UntypedJoin("b")
	bFile, err := bPath.Create()
	assert.NilError(t, err, "Create")
	_, err = bFile.WriteString("bFile")
	assert.NilError(t, err, "WriteString")
	assert.NilError(t, bFile.Close(), "Close")

	srcLinkPath := childDir.UntypedJoin("link")
	linkTarget := filepath.FromSlash("../b")
	assert.NilError(t, srcLinkPath.Symlink(linkTarget), "Symlink")

	srcBrokenLinkPath := childDir.Join("broken")
	assert.NilError(t, srcBrokenLinkPath.Symlink("missing"), "Symlink")
	circlePath := childDir.Join("circle")
	assert.NilError(t, circlePath.Symlink(filepath.FromSlash("../child")), "Symlink")

	files := []turbopath.AnchoredSystemPath{
		turbopath.AnchoredUnixPath(".").ToSystemPath(),            // src
		turbopath.AnchoredUnixPath("child/").ToSystemPath(),       // childDir
		turbopath.AnchoredUnixPath("child/a").ToSystemPath(),      // aPath,
		turbopath.AnchoredUnixPath("b").ToSystemPath(),            // bPath,
		turbopath.AnchoredUnixPath("child/link").ToSystemPath(),   // srcLinkPath,
		turbopath.AnchoredUnixPath("child/broken").ToSystemPath(), // srcBrokenLinkPath,
		turbopath.AnchoredUnixPath("child/circle").ToSystemPath(), // circlePath
	}

	dst := turbopath.AbsoluteSystemPath(t.TempDir())
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
	dstCachePath := dst.UntypedJoin(hash)

	dstAPath := dstCachePath.UntypedJoin("child", "a")
	assertFileMatches(t, aPath.ToStringDuringMigration(), dstAPath.ToStringDuringMigration())

	dstBPath := dstCachePath.UntypedJoin("b")
	assertFileMatches(t, bPath.ToStringDuringMigration(), dstBPath.ToStringDuringMigration())

	dstLinkPath := dstCachePath.UntypedJoin("child", "link")
	target, err := dstLinkPath.Readlink()
	assert.NilError(t, err, "Readlink")
	if target != linkTarget {
		t.Errorf("Readlink got %v, want %v", target, linkTarget)
	}

	dstBrokenLinkPath := dstCachePath.UntypedJoin("child", "broken")
	target, err = dstBrokenLinkPath.Readlink()
	assert.NilError(t, err, "Readlink")
	if target != "missing" {
		t.Errorf("Readlink got %v, want missing", target)
	}

	dstCirclePath := dstCachePath.UntypedJoin("child", "circle")
	circleLinkDest, err := dstCirclePath.Readlink()
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

	// cwd, err := fs.GetCwd()
	// assert.NilError(t, err, "GetCwd")
	cacheDir := turbopath.AbsoluteSystemPath(t.TempDir())
	src := cacheDir.UntypedJoin("the-hash", "some-package")
	err := src.MkdirAll()
	assert.NilError(t, err, "mkdirAll")

	childDir := src.UntypedJoin("child")
	err = childDir.MkdirAll()
	assert.NilError(t, err, "Mkdir")
	aPath := childDir.UntypedJoin("a")
	aFile, err := aPath.Create()
	assert.NilError(t, err, "Create")
	_, err = aFile.WriteString("hello")
	assert.NilError(t, err, "WriteString")
	assert.NilError(t, aFile.Close(), "Close")

	bPath := src.UntypedJoin("b")
	bFile, err := bPath.Create()
	assert.NilError(t, err, "Create")
	_, err = bFile.WriteString("bFile")
	assert.NilError(t, err, "WriteString")
	assert.NilError(t, bFile.Close(), "Close")

	srcLinkPath := childDir.UntypedJoin("link")
	linkTarget := filepath.FromSlash("../b")
	assert.NilError(t, srcLinkPath.Symlink(linkTarget), "Symlink")

	srcBrokenLinkPath := childDir.UntypedJoin("broken")
	assert.NilError(t, srcBrokenLinkPath.Symlink("missing"), "Symlink")
	circlePath := childDir.Join("circle")
	assert.NilError(t, circlePath.Symlink(filepath.FromSlash("../child")), "Symlink")

	metadataPath := cacheDir.UntypedJoin("the-hash-meta.json")
	err = metadataPath.WriteFile([]byte(`{"hash":"the-hash","duration":0}`), 0777)
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

	outputDir := turbopath.AbsoluteSystemPath(t.TempDir())
	dstOutputPath := "some-package"
	hit, files, _, err := cache.Fetch(outputDir.ToStringDuringMigration(), "the-hash", []string{})
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

	dstAPath := filepath.Join(outputDir.ToStringDuringMigration(), dstOutputPath, "child", "a")
	assertFileMatches(t, aPath.ToStringDuringMigration(), dstAPath)

	dstBPath := filepath.Join(outputDir.ToStringDuringMigration(), dstOutputPath, "b")
	assertFileMatches(t, bPath.ToStringDuringMigration(), dstBPath)

	dstLinkPath := filepath.Join(outputDir.ToStringDuringMigration(), dstOutputPath, "child", "link")
	target, err := os.Readlink(dstLinkPath)
	assert.NilError(t, err, "Readlink")
	if target != linkTarget {
		t.Errorf("Readlink got %v, want %v", target, linkTarget)
	}

	// We currently don't restore broken symlinks. This is probably a bug
	dstBrokenLinkPath := filepath.Join(outputDir.ToStringDuringMigration(), dstOutputPath, "child", "broken")
	_, err = os.Readlink(dstBrokenLinkPath)
	assert.ErrorIs(t, err, os.ErrNotExist)

	// Currently, on restore, we convert symlink-to-directory to empty-directory
	// This is very likely not ideal behavior, but leaving this test here to verify
	// that it is what we expect at this point in time.
	dstCirclePath := filepath.Join(outputDir.ToStringDuringMigration(), dstOutputPath, "child", "circle")
	circleStat, err := os.Lstat(dstCirclePath)
	assert.NilError(t, err, "Lstat")
	assert.Equal(t, circleStat.IsDir(), true)
	entries, err := os.ReadDir(dstCirclePath)
	assert.NilError(t, err, "ReadDir")
	assert.Equal(t, len(entries), 0)
}
