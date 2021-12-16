package globby

import (
	"os"
	"path/filepath"
	"reflect"
	"sort"
	"testing"
)

/*
 *  Test ignore .git
 */
func TestIgnoreDotGitFiles(t *testing.T) {
	// Init test files
	curDir, _ := os.Getwd()
	tmpDir := filepath.Join(curDir, "./tmp")
	defer os.RemoveAll(tmpDir)
	makeTmpFiles(tmpDir, []string{
		".git/file",
		".gitignore",
		"app.js",
	})

	// Match the patterns
	files := Match([]string{"."}, Option{BaseDir: tmpDir})
	// Expected match files:
	expected := []string{"app.js"}
	if checkFiles(tmpDir, files, expected) {
		t.Errorf("files not match, expected %v, but got %v", expected, files)
	}
}

/*
 *  Match "./** /*.jpg"
 */
func TestMathAllImg(t *testing.T) {
	// Init test files
	curDir, _ := os.Getwd()
	tmpDir := filepath.Join(curDir, "./tmp")
	defer os.RemoveAll(tmpDir)
	makeTmpFiles(tmpDir, []string{
		"app.js",
		"src/test.js",
		"image/footer.jpg",
		"image/logo.jpg",
		"image/user/avatar.jpg",
	})
	// Match the patterns
	files := Match([]string{
		"./**/*.jpg",
	}, Option{BaseDir: tmpDir})
	// Expected match files:
	expected := []string{
		"image/footer.jpg",
		"image/logo.jpg",
		"image/user/avatar.jpg",
	}
	if checkFiles(tmpDir, files, expected) {
		t.Errorf("files not match, expected %v, but got %v", expected, files)
	}
}

/*
 *  Match "src/*.js"
 */
func TestSignleStarFiles(t *testing.T) {
	// Init test files
	curDir, _ := os.Getwd()
	tmpDir := filepath.Join(curDir, "./tmp")
	defer os.RemoveAll(tmpDir)
	makeTmpFiles(tmpDir, []string{
		".git",
		"app.js",
		"package.json",
		"src/router.js",
		"src/store.js",
		"src/api/home.js",
		"src/api/user.js",
		"src/api/test.js",
	})

	patterns := []string{
		"src/*.js",
	}

	// Match the patterns
	files := Match(patterns, Option{BaseDir: tmpDir})
	// Expected match files:
	expected := []string{
		"src/router.js",
		"src/store.js",
	}
	if checkFiles(tmpDir, files, expected) {
		t.Errorf("files not match, expected %v, but got %v", expected, files)
	}
}

/*
 *  Match "src/api"
 */
func TestDirMatch(t *testing.T) {
	// Init test files
	curDir, _ := os.Getwd()
	tmpDir := filepath.Join(curDir, "./tmp")
	defer os.RemoveAll(tmpDir)
	makeTmpFiles(tmpDir, []string{
		".git",
		"app.js",
		"package.json",
		"src/router.js",
		"src/store.js",
		"src/api/home.js",
		"src/api/user.js",
		"src/api/test.js",
	})

	patterns := []string{
		"src/api",
	}

	// Match the patterns
	files := Match(patterns, Option{BaseDir: tmpDir})
	// Expected match files:
	expected := []string{
		"src/api/home.js",
		"src/api/user.js",
		"src/api/test.js",
	}
	if checkFiles(tmpDir, files, expected) {
		t.Errorf("files not match, expected %v, but got %v", expected, files)
	}
}

/*
 *  Match "/**" + "/*"
 */
func TestDirStar(t *testing.T) {
	// Init test files
	curDir, _ := os.Getwd()
	tmpDir := filepath.Join(curDir, "./tmp")
	defer os.RemoveAll(tmpDir)
	makeTmpFiles(tmpDir, []string{
		".git",
		"app.js",
		"package.json",
		"src/router.js",
		"src/store.js",
		"src/api/home.js",
		"src/api/user.js",
		"src/api/test.js",
	})

	patterns := []string{
		"src/**/*",
	}

	// Match the patterns
	files := Match(patterns, Option{BaseDir: tmpDir})
	// Expected match files:
	expected := []string{
		"src/router.js",
		"src/store.js",
		"src/api/home.js",
		"src/api/user.js",
		"src/api/test.js",
	}
	if checkFiles(tmpDir, files, expected) {
		t.Errorf("files not match, expected %v, but got %v", expected, files)
	}
}

/*
 *  Match "/**" + "/*.js"
 */
func TestDirStar2(t *testing.T) {
	// Init test files
	curDir, _ := os.Getwd()
	tmpDir := filepath.Join(curDir, "./tmp")
	defer os.RemoveAll(tmpDir)
	makeTmpFiles(tmpDir, []string{
		".git",
		"app.js",
		"package.json",
		"src/router.js",
		"src/store.js",
		"src/store.ts",
		"src/api/home.js",
		"src/api/user.js",
		"src/api/test.js",
	})

	patterns := []string{
		"src/**/*.js",
	}

	// Match the patterns
	files := Match(patterns, Option{BaseDir: tmpDir})
	// Expected match files:
	expected := []string{
		"src/router.js",
		"src/store.js",
		"src/api/home.js",
		"src/api/user.js",
		"src/api/test.js",
	}
	if checkFiles(tmpDir, files, expected) {
		t.Errorf("files not match, expected %v, but got %v", expected, files)
	}
}

/*
 * Match "/**" + "/*.js"
 * ignore files in the match items
 */
func TestDirIgnoreFile(t *testing.T) {
	// Init test files
	curDir, _ := os.Getwd()
	tmpDir := filepath.Join(curDir, "./tmp")
	defer os.RemoveAll(tmpDir)
	makeTmpFiles(tmpDir, []string{
		".git",
		"app.js",
		"package.json",
		"src/router.js",
		"src/store.js",
		"src/store.ts",
		"src/api/home.js",
		"src/api/user.js",
		"src/api/test.js",
		"src/service/home.js",
		"src/service/user.js",
		"src/service/test.js",
	})

	patterns := []string{
		"src/**/*.js",
		"!src/service/home.js",
	}

	// Match the patterns
	files := Match(patterns, Option{BaseDir: tmpDir})
	// Expected match files:
	expected := []string{
		"src/router.js",
		"src/store.js",
		"src/api/home.js",
		"src/api/user.js",
		"src/api/test.js",
		"src/service/user.js",
		"src/service/test.js",
	}
	if checkFiles(tmpDir, files, expected) {
		t.Errorf("files not match, expected %v, but got %v", expected, files)
	}
}

/*
 * Match "/**" + "/*.js"
 * ignore dir in the match items
 */
func TestDirIgnoreDir(t *testing.T) {
	// Init test files
	curDir, _ := os.Getwd()
	tmpDir := filepath.Join(curDir, "./tmp")
	defer os.RemoveAll(tmpDir)
	makeTmpFiles(tmpDir, []string{
		".git",
		"app.js",
		"package.json",
		"src/router.js",
		"src/store.js",
		"src/store.ts",
		"src/api/home.js",
		"src/api/user.js",
		"src/api/test.js",
		"src/service/home.js",
		"src/service/user.js",
		"src/service/test.js",
	})

	patterns := []string{
		"src/**/*.js",
		"!src/service",
	}

	// Match the patterns
	files := Match(patterns, Option{BaseDir: tmpDir})
	// Expected match files:
	expected := []string{
		"src/router.js",
		"src/store.js",
		"src/api/home.js",
		"src/api/user.js",
		"src/api/test.js",
	}
	if checkFiles(tmpDir, files, expected) {
		t.Errorf("files not match, expected %v, but got %v", expected, files)
	}
}

func TestMain(m *testing.M) {
	os.Exit(m.Run())
}

func makeTmpFiles(baseDir string, files []string) {
	for _, file := range files {
		file = filepath.Join(baseDir, file)
		dir, _ := filepath.Split(file)
		os.MkdirAll(dir, os.ModePerm)
		os.OpenFile(file, os.O_RDONLY|os.O_CREATE, 0666)
	}
}

func checkFiles(baseDir string, resultFiles []string, expectedFiles []string) bool {
	var expected []string
	for _, file := range expectedFiles {
		expected = append(expected, filepath.Join(baseDir, file))
	}
	sort.Sort(sort.Reverse(sort.StringSlice(resultFiles)))
	sort.Sort(sort.Reverse(sort.StringSlice(expected)))
	return !reflect.DeepEqual(resultFiles, expected)
}
