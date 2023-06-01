package cache

import (
	"archive/tar"
	"bytes"
	"errors"
	"net/http"
	"os"
	"testing"

	"github.com/DataDog/zstd"
	"github.com/vercel/turbo/cli/internal/cacheitem"
	"github.com/vercel/turbo/cli/internal/fs"
	"github.com/vercel/turbo/cli/internal/turbopath"
	"github.com/vercel/turbo/cli/internal/util"
	"gotest.tools/v3/assert"
)

type errorResp struct {
	err error
	t   *testing.T
}

func (sr *errorResp) PutArtifact(hash string, body []byte, duration int, tag string) error {
	sr.t.Helper()
	outdir := turbopath.AbsoluteSystemPathFromUpstream(sr.t.TempDir())
	cache := cacheitem.FromReader(bytes.NewReader(body), true)
	restored, err := cache.Restore(outdir)

	sr.t.Log(restored)
	assert.Equal(sr.t, restored[0].ToString(), "one")
	assert.Equal(sr.t, restored[1].ToString(), "two")
	assert.Equal(sr.t, len(restored), 2)
	assert.NilError(sr.t, err, "Restoration was successful.")

	return sr.err
}

func (sr *errorResp) FetchArtifact(hash string) (*http.Response, error) {
	return nil, sr.err
}

func (sr *errorResp) ArtifactExists(hash string) (*http.Response, error) {
	return nil, sr.err
}

func (sr *errorResp) GetTeamID() string {
	return ""
}

func TestRemoteCachingDisabled(t *testing.T) {
	clientErr := &util.CacheDisabledError{
		Status:  util.CachingStatusDisabled,
		Message: "Remote Caching has been disabled for this team. A team owner can enable it here: $URL",
	}
	client := &errorResp{err: clientErr}
	cache := &httpCache{
		client:         client,
		requestLimiter: make(limiter, 20),
	}
	cd := &util.CacheDisabledError{}
	_, _, err := cache.Fetch("unused-target", "some-hash", []string{"unused", "outputs"})
	if !errors.As(err, &cd) {
		t.Errorf("cache.Fetch err got %v, want a CacheDisabled error", err)
	}
	if cd.Status != util.CachingStatusDisabled {
		t.Errorf("CacheDisabled.Status got %v, want %v", cd.Status, util.CachingStatusDisabled)
	}
}

func makeValidTar(t *testing.T) *bytes.Buffer {
	// <repoRoot>
	//   my-pkg/
	//     some-file
	//     link-to-extra-file -> ../extra-file
	//     broken-link -> ../../global-dep
	//   extra-file

	t.Helper()
	buf := &bytes.Buffer{}
	zw := zstd.NewWriter(buf)
	defer func() {
		if err := zw.Close(); err != nil {
			t.Fatalf("failed to close gzip: %v", err)
		}
	}()
	tw := tar.NewWriter(zw)
	defer func() {
		if err := tw.Close(); err != nil {
			t.Fatalf("failed to close tar: %v", err)
		}
	}()

	// my-pkg
	h := &tar.Header{
		Name:     "my-pkg/",
		Mode:     int64(0755),
		Typeflag: tar.TypeDir,
	}
	if err := tw.WriteHeader(h); err != nil {
		t.Fatalf("failed to write header: %v", err)
	}
	// my-pkg/some-file
	contents := []byte("some-file-contents")
	h = &tar.Header{
		Name:     "my-pkg/some-file",
		Mode:     int64(0644),
		Typeflag: tar.TypeReg,
		Size:     int64(len(contents)),
	}
	if err := tw.WriteHeader(h); err != nil {
		t.Fatalf("failed to write header: %v", err)
	}
	if _, err := tw.Write(contents); err != nil {
		t.Fatalf("failed to write file: %v", err)
	}
	// my-pkg/link-to-extra-file
	h = &tar.Header{
		Name:     "my-pkg/link-to-extra-file",
		Mode:     int64(0644),
		Typeflag: tar.TypeSymlink,
		Linkname: "../extra-file",
	}
	if err := tw.WriteHeader(h); err != nil {
		t.Fatalf("failed to write header: %v", err)
	}
	// my-pkg/broken-link
	h = &tar.Header{
		Name:     "my-pkg/broken-link",
		Mode:     int64(0644),
		Typeflag: tar.TypeSymlink,
		Linkname: "../../global-dep",
	}
	if err := tw.WriteHeader(h); err != nil {
		t.Fatalf("failed to write header: %v", err)
	}
	// extra-file
	contents = []byte("extra-file-contents")
	h = &tar.Header{
		Name:     "extra-file",
		Mode:     int64(0644),
		Typeflag: tar.TypeReg,
		Size:     int64(len(contents)),
	}
	if err := tw.WriteHeader(h); err != nil {
		t.Fatalf("failed to write header: %v", err)
	}
	if _, err := tw.Write(contents); err != nil {
		t.Fatalf("failed to write file: %v", err)
	}

	return buf
}

func makeInvalidTar(t *testing.T) *bytes.Buffer {
	// contains a single file that traverses out
	// ../some-file

	t.Helper()
	buf := &bytes.Buffer{}
	zw := zstd.NewWriter(buf)
	defer func() {
		if err := zw.Close(); err != nil {
			t.Fatalf("failed to close gzip: %v", err)
		}
	}()
	tw := tar.NewWriter(zw)
	defer func() {
		if err := tw.Close(); err != nil {
			t.Fatalf("failed to close tar: %v", err)
		}
	}()

	// my-pkg/some-file
	contents := []byte("some-file-contents")
	h := &tar.Header{
		Name:     "../some-file",
		Mode:     int64(0644),
		Typeflag: tar.TypeReg,
		Size:     int64(len(contents)),
	}
	if err := tw.WriteHeader(h); err != nil {
		t.Fatalf("failed to write header: %v", err)
	}
	if _, err := tw.Write(contents); err != nil {
		t.Fatalf("failed to write file: %v", err)
	}
	return buf
}

func TestRestoreTar(t *testing.T) {
	root := fs.AbsoluteSystemPathFromUpstream(t.TempDir())

	tar := makeValidTar(t)

	expectedFiles := []turbopath.AnchoredSystemPath{
		turbopath.AnchoredUnixPath("extra-file").ToSystemPath(),
		turbopath.AnchoredUnixPath("my-pkg").ToSystemPath(),
		turbopath.AnchoredUnixPath("my-pkg/some-file").ToSystemPath(),
		turbopath.AnchoredUnixPath("my-pkg/link-to-extra-file").ToSystemPath(),
		turbopath.AnchoredUnixPath("my-pkg/broken-link").ToSystemPath(),
	}
	files, err := restoreTar(root, tar)
	assert.NilError(t, err, "readTar")

	expectedSet := make(util.Set)
	for _, file := range expectedFiles {
		expectedSet.Add(file.ToString())
	}
	gotSet := make(util.Set)
	for _, file := range files {
		gotSet.Add(file.ToString())
	}
	extraFiles := gotSet.Difference(expectedSet)
	if extraFiles.Len() > 0 {
		t.Errorf("got extra files: %v", extraFiles.UnsafeListOfStrings())
	}
	missingFiles := expectedSet.Difference(gotSet)
	if missingFiles.Len() > 0 {
		t.Errorf("missing expected files: %v", missingFiles.UnsafeListOfStrings())
	}

	// Verify file contents
	extraFile := root.UntypedJoin("extra-file")
	contents, err := extraFile.ReadFile()
	assert.NilError(t, err, "ReadFile")
	assert.DeepEqual(t, contents, []byte("extra-file-contents"))

	someFile := root.UntypedJoin("my-pkg", "some-file")
	contents, err = someFile.ReadFile()
	assert.NilError(t, err, "ReadFile")
	assert.DeepEqual(t, contents, []byte("some-file-contents"))
}

func TestRestoreInvalidTar(t *testing.T) {
	root := fs.AbsoluteSystemPathFromUpstream(t.TempDir())
	expectedContents := []byte("important-data")
	someFile := root.UntypedJoin("some-file")
	err := someFile.WriteFile(expectedContents, 0644)
	assert.NilError(t, err, "WriteFile")

	tar := makeInvalidTar(t)
	// use a child directory so that blindly untarring will squash the file
	// that we just wrote above.
	repoRoot := root.UntypedJoin("repo")
	_, err = restoreTar(repoRoot, tar)
	if err == nil {
		t.Error("expected error untarring invalid tar")
	}

	contents, err := someFile.ReadFile()
	assert.NilError(t, err, "ReadFile")
	assert.Equal(t, string(contents), string(expectedContents), "expected to not overwrite file")
}

func Test_httpCache_Put(t *testing.T) {
	root := fs.AbsoluteSystemPathFromUpstream(t.TempDir())
	_ = root.Join("one").WriteFile(nil, 0644)
	_ = root.Join("two").WriteFile(nil, 0644)

	clientErr := errors.New("PutArtifact")
	client := &errorResp{err: clientErr, t: t}

	cache := newHTTPCache(Opts{}, client, nil, root)

	assert.ErrorIs(
		t,
		cache.Put(root, "000", 10, []turbopath.AnchoredSystemPath{"one", "two"}),
		clientErr,
		"Succeeds at writing, cache item is successfully passed through.",
	)

	assert.ErrorIs(
		t,
		cache.Put(root, "000", 10, []turbopath.AnchoredSystemPath{"one", "two", "missing"}),
		os.ErrNotExist,
		"Errors with missing file.",
	)

	assert.ErrorIs(
		t,
		cache.Put(root, "000", 10, []turbopath.AnchoredSystemPath{"missing", "one", "two"}),
		os.ErrNotExist,
		"Errors with missing file at first load.",
	)
}
