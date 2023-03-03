package taskhash

import (
	"path/filepath"
	"strings"
	"testing"

	"github.com/vercel/turbo/cli/internal/fs"
	"github.com/vercel/turbo/cli/internal/turbopath"
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
	root := t.TempDir()
	repoRoot := turbopath.AbsoluteSystemPathFromUpstream(root)
	pkgName := turbopath.AnchoredUnixPath("child-dir/libA").ToSystemPath()
	type fileHash struct {
		contents string
		hash     string
	}
	files := map[turbopath.AnchoredUnixPath]fileHash{
		"top-level-file":                        {"top-level-file-contents", ""},
		"other-dir/other-dir-file":              {"other-dir-file-contents", ""},
		"ignoreme":                              {"anything", ""},
		"child-dir/libA/some-file":              {"some-file-contents", "7e59c6a6ea9098c6d3beb00e753e2c54ea502311"},
		"child-dir/libA/some-dir/other-file":    {"some-file-contents", "7e59c6a6ea9098c6d3beb00e753e2c54ea502311"},
		"child-dir/libA/some-dir/another-one":   {"some-file-contents", "7e59c6a6ea9098c6d3beb00e753e2c54ea502311"},
		"child-dir/libA/some-dir/excluded-file": {"some-file-contents", "7e59c6a6ea9098c6d3beb00e753e2c54ea502311"},
		"child-dir/libA/ignoreme":               {"anything", ""},
		"child-dir/libA/ignorethisdir/anything": {"anything", ""},
		"child-dir/libA/pkgignoreme":            {"anything", ""},
		"child-dir/libA/pkgignorethisdir/file":  {"anything", ""},
	}

	rootIgnoreFile, err := repoRoot.Join(".gitignore").Create()
	if err != nil {
		t.Fatalf("failed to create .gitignore: %v", err)
	}
	_, err = rootIgnoreFile.WriteString(rootIgnore)
	if err != nil {
		t.Fatalf("failed to write contents to .gitignore: %v", err)
	}
	rootIgnoreFile.Close()
	pkgIgnoreFilename := pkgName.RestoreAnchor(repoRoot).Join(".gitignore")
	err = pkgIgnoreFilename.EnsureDir()
	if err != nil {
		t.Fatalf("failed to ensure directories for %v: %v", pkgIgnoreFilename, err)
	}
	pkgIgnoreFile, err := pkgIgnoreFilename.Create()
	if err != nil {
		t.Fatalf("failed to create libA/.gitignore: %v", err)
	}
	_, err = pkgIgnoreFile.WriteString(pkgIgnore)
	if err != nil {
		t.Fatalf("failed to write contents to libA/.gitignore: %v", err)
	}
	pkgIgnoreFile.Close()
	for path, spec := range files {
		filename := path.ToSystemPath().RestoreAnchor(repoRoot)
		err = filename.EnsureDir()
		if err != nil {
			t.Fatalf("failed to ensure directories for %v: %v", filename, err)
		}
		f, err := filename.Create()
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
	files[turbopath.AnchoredUnixPath("child-dir/libA/.gitignore")] = fileHash{contents: "", hash: "3237694bc3312ded18386964a855074af7b066af"}

	pkg := &fs.PackageJSON{
		Dir: pkgName,
	}
	hashes, err := manuallyHashPackage(pkg, []string{}, repoRoot)
	if err != nil {
		t.Fatalf("failed to calculate manual hashes: %v", err)
	}

	count := 0
	for path, spec := range files {
		systemPath := path.ToSystemPath()
		if systemPath.HasPrefix(pkgName) {
			relPath := systemPath[len(pkgName)+1:]
			got, ok := hashes[relPath.ToUnixPath()]
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
	justFileHashes, err := manuallyHashPackage(pkg, []string{filepath.FromSlash("**/*file"), "!" + filepath.FromSlash("some-dir/excluded-file")}, repoRoot)
	if err != nil {
		t.Fatalf("failed to calculate manual hashes: %v", err)
	}
	for path, spec := range files {
		systemPath := path.ToSystemPath()
		if systemPath.HasPrefix(pkgName) {
			shouldInclude := strings.HasSuffix(systemPath.ToString(), "file") && !strings.HasSuffix(systemPath.ToString(), "excluded-file")
			relPath := systemPath[len(pkgName)+1:]
			got, ok := justFileHashes[relPath.ToUnixPath()]
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
