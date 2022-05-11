package run

import (
	"os"
	"path/filepath"
	"strings"
	"testing"

	"github.com/vercel/turborepo/cli/internal/fs"
)

func Test_manuallyHashPackage(t *testing.T) {
	rootIgnore := strings.Join([]string{
		"ignoreme",
		"ignorethisdir/",
	}, "\n")
	pkgIgnore := strings.Join([]string{
		"pkgignoreme",
		"pkgignorethisdir/",
	}, "\n")
	root, err := os.MkdirTemp("", "turbo-manual-file-hashing-")
	if err != nil {
		t.Fatalf("failed to create temp dir: %v", err)
	}
	repoRoot := fs.UnsafeToAbsolutePath(root)
	pkgName := "libA"
	type fileHash struct {
		contents string
		hash     string
	}
	files := map[string]fileHash{
		filepath.FromSlash("top-level-file"):              {"top-level-file-contents", ""},
		filepath.FromSlash("other-dir/other-dir-file"):    {"other-dir-file-contents", ""},
		filepath.FromSlash("ignoreme"):                    {"anything", ""},
		filepath.FromSlash("libA/some-file"):              {"some-file-contents", "7e59c6a6ea9098c6d3beb00e753e2c54ea502311"},
		filepath.FromSlash("libA/some-dir/other-file"):    {"some-file-contents", "7e59c6a6ea9098c6d3beb00e753e2c54ea502311"},
		filepath.FromSlash("libA/some-dir/another-one"):   {"some-file-contents", "7e59c6a6ea9098c6d3beb00e753e2c54ea502311"},
		filepath.FromSlash("libA/ignoreme"):               {"anything", ""},
		filepath.FromSlash("libA/ignorethisdir/anything"): {"anything", ""},
		filepath.FromSlash("libA/pkgignoreme"):            {"anything", ""},
		filepath.FromSlash("libA/pkgignorethisdir/file"):  {"anything", ""},
	}

	rootIgnoreFile, err := os.Create(filepath.Join(root, ".gitignore"))
	if err != nil {
		t.Fatalf("failed to create .gitignore: %v", err)
	}
	_, err = rootIgnoreFile.WriteString(rootIgnore)
	if err != nil {
		t.Fatalf("failed to write contents to .gitignore: %v", err)
	}
	rootIgnoreFile.Close()
	pkgIgnoreFilename := filepath.Join(root, pkgName, ".gitignore")
	err = fs.EnsureDir(pkgIgnoreFilename)
	if err != nil {
		t.Fatalf("failed to ensure directories for %v: %v", pkgIgnoreFilename, err)
	}
	pkgIgnoreFile, err := os.Create(pkgIgnoreFilename)
	if err != nil {
		t.Fatalf("failed to create libA/.gitignore: %v", err)
	}
	_, err = pkgIgnoreFile.WriteString(pkgIgnore)
	if err != nil {
		t.Fatalf("failed to write contents to libA/.gitignore: %v", err)
	}
	pkgIgnoreFile.Close()
	for path, spec := range files {
		filename := filepath.Join(root, path)
		err = fs.EnsureDir(filename)
		if err != nil {
			t.Fatalf("failed to ensure directories for %v: %v", filename, err)
		}
		f, err := os.Create(filename)
		if err != nil {
			t.Fatalf("failed to create file: %v: %v", filename, err)
		}
		_, err = f.WriteString(spec.contents)
		if err != nil {
			t.Fatalf("failed to write contents to %v: %v", filename, err)
		}
		f.Close()
	}
	// now that we've created the repo, expect our .gitignore file too
	files[filepath.FromSlash("libA/.gitignore")] = fileHash{contents: "", hash: "3237694bc3312ded18386964a855074af7b066af"}

	pkg := &fs.PackageJSON{
		Dir: pkgName,
	}
	hashes, err := manuallyHashPackage(pkg, []string{}, repoRoot)
	if err != nil {
		t.Fatalf("failed to calculate manual hashes: %v", err)
	}
	prefix := filepath.FromSlash(pkgName + "/")
	prefixLen := len(prefix)
	count := 0
	for path, spec := range files {
		if strings.HasPrefix(path, prefix) {
			got, ok := hashes[path[prefixLen:]]
			if !ok {
				if spec.hash != "" {
					t.Errorf("did not find hash for %v, but wanted one", path)
				}
			} else if got != spec.hash {
				t.Errorf("hash of %v, got %v want %v", path, got, spec.hash)
			} else {
				count++
			}
		}
	}
	if count != len(hashes) {
		t.Errorf("found extra hashes in %v", hashes)
	}

	count = 0
	justFileHashes, err := manuallyHashPackage(pkg, []string{filepath.FromSlash("**/*file")}, repoRoot)
	if err != nil {
		t.Fatalf("failed to calculate manual hashes: %v", err)
	}
	for path, spec := range files {
		if strings.HasPrefix(path, prefix) {
			shouldInclude := strings.HasSuffix(path, "file")
			got, ok := justFileHashes[path[prefixLen:]]
			if !ok && shouldInclude {
				if spec.hash != "" {
					t.Errorf("did not find hash for %v, but wanted one", path)
				}
			} else if shouldInclude && got != spec.hash {
				t.Errorf("hash of %v, got %v want %v", path, got, spec.hash)
			} else if shouldInclude {
				count++
			}
		}
	}
	if count != len(justFileHashes) {
		t.Errorf("found extra hashes in %v", hashes)
	}
}
