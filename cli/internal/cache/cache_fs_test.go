package cache

import (
	"path/filepath"
	"testing"

	"github.com/vercel/turborepo/cli/internal/analytics"
	"github.com/vercel/turborepo/cli/internal/cacheitem"
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
	err := childDir.MkdirAll(0775)
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
		turbopath.AnchoredUnixPath("child/").ToSystemPath(),       // childDir
		turbopath.AnchoredUnixPath("child/a").ToSystemPath(),      // aPath,
		turbopath.AnchoredUnixPath("b").ToSystemPath(),            // bPath,
		turbopath.AnchoredUnixPath("child/link").ToSystemPath(),   // srcLinkPath,
		turbopath.AnchoredUnixPath("child/broken").ToSystemPath(), // srcBrokenLinkPath,
		turbopath.AnchoredUnixPath("child/circle").ToSystemPath(), // circlePath
	}

	dst := turbopath.AbsoluteSystemPath(t.TempDir())
	dr := &dummyRecorder{}

	cache := &fsCache{
		cacheDirectory: dst,
		recorder:       dr,
	}

	hash := "the-hash"
	duration := 0
	putErr := cache.Put(src, hash, duration, files)
	assert.NilError(t, putErr, "Put")

	// Verify that we got the files that we're expecting
	dstCachePath := dst.UntypedJoin(hash)

	// This test checks outputs, so we go ahead and pull things back out.
	// Attempting to satisfy our beliefs that the change is viable with
	// as few changes to the tests as possible.
	cacheItem, openErr := cacheitem.Open(dst.UntypedJoin(hash + ".tar.zst"))
	assert.NilError(t, openErr, "Open")

	_, restoreErr := cacheItem.Restore(dstCachePath)
	assert.NilError(t, restoreErr, "Restore")

	dstAPath := dstCachePath.UntypedJoin("child", "a")
	assertFileMatches(t, aPath, dstAPath)

	dstBPath := dstCachePath.UntypedJoin("b")
	assertFileMatches(t, bPath, dstBPath)

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

	assert.NilError(t, cacheItem.Close(), "Close")
}

func assertFileMatches(t *testing.T, orig turbopath.AbsoluteSystemPath, copy turbopath.AbsoluteSystemPath) {
	t.Helper()
	origBytes, err := orig.ReadFile()
	assert.NilError(t, err, "ReadFile")
	copyBytes, err := copy.ReadFile()
	assert.NilError(t, err, "ReadFile")
	assert.DeepEqual(t, origBytes, copyBytes)
	origStat, err := orig.Lstat()
	assert.NilError(t, err, "Lstat")
	copyStat, err := copy.Lstat()
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

	cacheDir := turbopath.AbsoluteSystemPath(t.TempDir())
	hash := "the-hash"
	src := cacheDir.UntypedJoin(hash, "some-package")
	err := src.MkdirAll(0775)
	assert.NilError(t, err, "mkdirAll")

	childDir := src.UntypedJoin("child")
	err = childDir.MkdirAll(0775)
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
	srcBrokenLinkTarget := turbopath.AnchoredUnixPath("missing").ToSystemPath()
	assert.NilError(t, srcBrokenLinkPath.Symlink(srcBrokenLinkTarget.ToString()), "Symlink")

	circlePath := childDir.Join("circle")
	srcCircleLinkTarget := turbopath.AnchoredUnixPath("../child").ToSystemPath()
	assert.NilError(t, circlePath.Symlink(srcCircleLinkTarget.ToString()), "Symlink")

	metadataPath := cacheDir.UntypedJoin("the-hash-meta.json")
	err = metadataPath.WriteFile([]byte(`{"hash":"the-hash","duration":0}`), 0777)
	assert.NilError(t, err, "WriteFile")

	dr := &dummyRecorder{}

	cache := &fsCache{
		cacheDirectory: cacheDir,
		recorder:       dr,
	}

	inputFiles := []turbopath.AnchoredSystemPath{
		turbopath.AnchoredUnixPath("some-package/child/").ToSystemPath(),       // childDir
		turbopath.AnchoredUnixPath("some-package/child/a").ToSystemPath(),      // aPath,
		turbopath.AnchoredUnixPath("some-package/b").ToSystemPath(),            // bPath,
		turbopath.AnchoredUnixPath("some-package/child/link").ToSystemPath(),   // srcLinkPath,
		turbopath.AnchoredUnixPath("some-package/child/broken").ToSystemPath(), // srcBrokenLinkPath,
		turbopath.AnchoredUnixPath("some-package/child/circle").ToSystemPath(), // circlePath
	}

	putErr := cache.Put(cacheDir.UntypedJoin(hash), hash, 0, inputFiles)
	assert.NilError(t, putErr, "Put")

	outputDir := turbopath.AbsoluteSystemPath(t.TempDir())
	dstOutputPath := "some-package"
	hit, files, _, err := cache.Fetch(outputDir, "the-hash", []string{})
	assert.NilError(t, err, "Fetch")
	if !hit {
		t.Error("Fetch got false, want true")
	}
	if len(files) != len(inputFiles) {
		t.Errorf("len(files) got %v, want %v", len(files), len(inputFiles))
	}

	dstAPath := outputDir.UntypedJoin(dstOutputPath, "child", "a")
	assertFileMatches(t, aPath, dstAPath)

	dstBPath := outputDir.UntypedJoin(dstOutputPath, "b")
	assertFileMatches(t, bPath, dstBPath)

	dstLinkPath := outputDir.UntypedJoin(dstOutputPath, "child", "link")
	target, err := dstLinkPath.Readlink()
	assert.NilError(t, err, "Readlink")
	if target != linkTarget {
		t.Errorf("Readlink got %v, want %v", target, linkTarget)
	}

	// Assert that we restore broken symlinks correctly
	dstBrokenLinkPath := outputDir.UntypedJoin(dstOutputPath, "child", "broken")
	target, readlinkErr := dstBrokenLinkPath.Readlink()
	assert.NilError(t, readlinkErr, "Readlink")
	assert.Equal(t, target, srcBrokenLinkTarget.ToString())

	// Assert that we restore symlinks to directories correctly
	dstCirclePath := outputDir.UntypedJoin(dstOutputPath, "child", "circle")
	circleTarget, circleReadlinkErr := dstCirclePath.Readlink()
	assert.NilError(t, circleReadlinkErr, "Circle Readlink")
	assert.Equal(t, circleTarget, srcCircleLinkTarget.ToString())
}
