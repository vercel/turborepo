package cache

import (
	"io/ioutil"
	"os"
	"path/filepath"
	"testing"

	"github.com/vercel/turborepo/cli/internal/analytics"
	turbofs "github.com/vercel/turborepo/cli/internal/fs"
	"gotest.tools/v3/assert"
)

type dummyRecorder struct{}

func (dr *dummyRecorder) LogEvent(payload analytics.EventPayload) {}

type testingUtil interface {
	Helper()
	Cleanup(f func())
}

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

	// TODO(gsoltis): doesn't work, because it implicitly runs from cwd
	files := []string{
		filepath.Join(src, filepath.FromSlash("child/a")),      // aPath,
		filepath.Join(src, "b"),                                // bPath,
		filepath.Join(src, filepath.FromSlash("child/link")),   // srcLinkPath,
		filepath.Join(src, filepath.FromSlash("child/broken")), // srcBrokenLinkPath,
	}

	dst := subdirForTest(t)
	dr := &dummyRecorder{}
	cache := &fsCache{
		cacheDirectory: dst,
		recorder:       dr,
	}

	hash := "the-hash"
	duration := 0
	err = cache.Put("unused", hash, duration, files)
	assert.NilError(t, err, "Put")

	dstCachePath := filepath.Join(dst, hash)

	dstAPath := filepath.Join(dstCachePath, src, "child", "a")
	got, err := turbofs.SameFile(aPath, dstAPath)
	assert.NilError(t, err, "SameFile")
	if !got {
		t.Errorf("SameFile(%v, %v) got false, want true", aPath, dstAPath)
	}

	dstBPath := filepath.Join(dstCachePath, src, "b")
	got, err = turbofs.SameFile(bPath, dstBPath)
	assert.NilError(t, err, "SameFile")
	if !got {
		t.Errorf("SameFile(%v, %v) got false, want true", bPath, dstBPath)
	}

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
}

func TestFetch(t *testing.T) {
	cacheDir := subdirForTest(t)
	src := filepath.Join(cacheDir, "the-hash", "some-package")
	err := os.MkdirAll(src, os.ModeDir|0777)
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
	cache := &fsCache{
		cacheDirectory: cacheDir,
		recorder:       dr,
	}

	deleteOnFinish(t, "the-output")
	hit, files, _, err := cache.Fetch("the-output", "the-hash", []string{})
	assert.NilError(t, err, "Fetch")
	if !hit {
		t.Error("Fetch got false, want true")
	}
	t.Logf("files %v", files)
}
