package globby

import (
	"io/fs"
	"path/filepath"
	"reflect"
	"sort"
	"testing"

	"testing/fstest"
)

// setup prepares the test file system contents and returns the file system.
func setup(fsysRoot string, files []string) fs.FS {
	fsys := fstest.MapFS{}
	for _, file := range files {
		// We're populating a `fs.FS` filesytem which requires paths to have no
		// leading slash. As a consequence we strip it during creation.
		iofsRelativePath := file[1:]

		fsys[iofsRelativePath] = &fstest.MapFile{Mode: 0666}
	}

	return fsys
}

func TestGlobFilesFs(t *testing.T) {
	type args struct {
		basePath        string
		includePatterns []string
		excludePatterns []string
	}
	tests := []struct {
		name      string
		files     []string
		args      args
		wantAll   []string
		wantFiles []string
		wantErr   bool
	}{
		{
			name:  "hello world",
			files: []string{"/test.txt"},
			args: args{
				basePath:        "/",
				includePatterns: []string{"*.txt"},
				excludePatterns: []string{},
			},
			wantAll:   []string{"/test.txt"},
			wantFiles: []string{"/test.txt"},
		},
		{
			name: "bullet files",
			files: []string{
				"/test.txt",
				"/subdir/test.txt",
				"/other/test.txt",
			},
			args: args{
				basePath:        "/",
				includePatterns: []string{"subdir/test.txt", "test.txt"},
				excludePatterns: []string{},
			},
			wantAll: []string{
				"/subdir/test.txt",
				"/test.txt",
			},
			wantFiles: []string{
				"/subdir/test.txt",
				"/test.txt",
			},
		},
		{
			name: "finding workspace package.json files",
			files: []string{
				"/external/file.txt",
				"/repos/some-app/apps/docs/package.json",
				"/repos/some-app/apps/web/package.json",
				"/repos/some-app/bower_components/readline/package.json",
				"/repos/some-app/examples/package.json",
				"/repos/some-app/node_modules/gulp/bower_components/readline/package.json",
				"/repos/some-app/node_modules/react/package.json",
				"/repos/some-app/package.json",
				"/repos/some-app/packages/colors/package.json",
				"/repos/some-app/packages/faker/package.json",
				"/repos/some-app/packages/left-pad/package.json",
				"/repos/some-app/test/mocks/kitchen-sink/package.json",
				"/repos/some-app/tests/mocks/kitchen-sink/package.json",
			},
			args: args{
				basePath:        "/repos/some-app/",
				includePatterns: []string{"packages/*/package.json", "apps/*/package.json"},
				excludePatterns: []string{"**/node_modules/", "**/bower_components/", "**/test/", "**/tests/"},
			},
			wantAll: []string{
				"/repos/some-app/apps/docs/package.json",
				"/repos/some-app/apps/web/package.json",
				"/repos/some-app/packages/colors/package.json",
				"/repos/some-app/packages/faker/package.json",
				"/repos/some-app/packages/left-pad/package.json",
			},
			wantFiles: []string{
				"/repos/some-app/apps/docs/package.json",
				"/repos/some-app/apps/web/package.json",
				"/repos/some-app/packages/colors/package.json",
				"/repos/some-app/packages/faker/package.json",
				"/repos/some-app/packages/left-pad/package.json",
			},
		},
		{
			name: "excludes unexpected workspace package.json files",
			files: []string{
				"/external/file.txt",
				"/repos/some-app/apps/docs/package.json",
				"/repos/some-app/apps/web/package.json",
				"/repos/some-app/bower_components/readline/package.json",
				"/repos/some-app/examples/package.json",
				"/repos/some-app/node_modules/gulp/bower_components/readline/package.json",
				"/repos/some-app/node_modules/react/package.json",
				"/repos/some-app/package.json",
				"/repos/some-app/packages/colors/package.json",
				"/repos/some-app/packages/faker/package.json",
				"/repos/some-app/packages/left-pad/package.json",
				"/repos/some-app/test/mocks/spanish-inquisition/package.json",
				"/repos/some-app/tests/mocks/spanish-inquisition/package.json",
			},
			args: args{
				basePath:        "/repos/some-app/",
				includePatterns: []string{"**/package.json"},
				excludePatterns: []string{"**/node_modules/", "**/bower_components/", "**/test/", "**/tests/"},
			},
			wantAll: []string{
				"/repos/some-app/apps/docs/package.json",
				"/repos/some-app/apps/web/package.json",
				"/repos/some-app/examples/package.json",
				"/repos/some-app/package.json",
				"/repos/some-app/packages/colors/package.json",
				"/repos/some-app/packages/faker/package.json",
				"/repos/some-app/packages/left-pad/package.json",
			},
			wantFiles: []string{
				"/repos/some-app/apps/docs/package.json",
				"/repos/some-app/apps/web/package.json",
				"/repos/some-app/examples/package.json",
				"/repos/some-app/package.json",
				"/repos/some-app/packages/colors/package.json",
				"/repos/some-app/packages/faker/package.json",
				"/repos/some-app/packages/left-pad/package.json",
			},
		},
		{
			name: "nested packages work",
			files: []string{
				"/external/file.txt",
				"/repos/some-app/apps/docs/package.json",
				"/repos/some-app/apps/web/package.json",
				"/repos/some-app/bower_components/readline/package.json",
				"/repos/some-app/examples/package.json",
				"/repos/some-app/node_modules/gulp/bower_components/readline/package.json",
				"/repos/some-app/node_modules/react/package.json",
				"/repos/some-app/package.json",
				"/repos/some-app/packages/xzibit/package.json",
				"/repos/some-app/packages/xzibit/node_modules/street-legal/package.json",
				"/repos/some-app/packages/xzibit/node_modules/paint-colors/package.json",
				"/repos/some-app/packages/xzibit/packages/yo-dawg/package.json",
				"/repos/some-app/packages/xzibit/packages/yo-dawg/node_modules/meme/package.json",
				"/repos/some-app/packages/xzibit/packages/yo-dawg/node_modules/yo-dawg/package.json",
				"/repos/some-app/packages/colors/package.json",
				"/repos/some-app/packages/faker/package.json",
				"/repos/some-app/packages/left-pad/package.json",
				"/repos/some-app/test/mocks/spanish-inquisition/package.json",
				"/repos/some-app/tests/mocks/spanish-inquisition/package.json",
			},
			args: args{
				basePath:        "/repos/some-app/",
				includePatterns: []string{"packages/**/package.json"},
				excludePatterns: []string{"**/node_modules/", "**/bower_components/", "**/test/", "**/tests/"},
			},
			wantAll: []string{
				"/repos/some-app/packages/colors/package.json",
				"/repos/some-app/packages/faker/package.json",
				"/repos/some-app/packages/left-pad/package.json",
				"/repos/some-app/packages/xzibit/package.json",
				"/repos/some-app/packages/xzibit/packages/yo-dawg/package.json",
			},
			wantFiles: []string{
				"/repos/some-app/packages/colors/package.json",
				"/repos/some-app/packages/faker/package.json",
				"/repos/some-app/packages/left-pad/package.json",
				"/repos/some-app/packages/xzibit/package.json",
				"/repos/some-app/packages/xzibit/packages/yo-dawg/package.json",
			},
		},
		{
			name: "includes do not override excludes",
			files: []string{
				"/external/file.txt",
				"/repos/some-app/apps/docs/package.json",
				"/repos/some-app/apps/web/package.json",
				"/repos/some-app/bower_components/readline/package.json",
				"/repos/some-app/examples/package.json",
				"/repos/some-app/node_modules/gulp/bower_components/readline/package.json",
				"/repos/some-app/node_modules/react/package.json",
				"/repos/some-app/package.json",
				"/repos/some-app/packages/xzibit/package.json",
				"/repos/some-app/packages/xzibit/node_modules/street-legal/package.json",
				"/repos/some-app/packages/xzibit/node_modules/paint-colors/package.json",
				"/repos/some-app/packages/xzibit/packages/yo-dawg/package.json",
				"/repos/some-app/packages/xzibit/packages/yo-dawg/node_modules/meme/package.json",
				"/repos/some-app/packages/xzibit/packages/yo-dawg/node_modules/yo-dawg/package.json",
				"/repos/some-app/packages/colors/package.json",
				"/repos/some-app/packages/faker/package.json",
				"/repos/some-app/packages/left-pad/package.json",
				"/repos/some-app/test/mocks/spanish-inquisition/package.json",
				"/repos/some-app/tests/mocks/spanish-inquisition/package.json",
			},
			args: args{
				basePath:        "/repos/some-app/",
				includePatterns: []string{"packages/**/package.json", "tests/mocks/*/package.json"},
				excludePatterns: []string{"**/node_modules/", "**/bower_components/", "**/test/", "**/tests/"},
			},
			wantAll: []string{
				"/repos/some-app/packages/colors/package.json",
				"/repos/some-app/packages/faker/package.json",
				"/repos/some-app/packages/left-pad/package.json",
				"/repos/some-app/packages/xzibit/package.json",
				"/repos/some-app/packages/xzibit/packages/yo-dawg/package.json",
			},
			wantFiles: []string{
				"/repos/some-app/packages/colors/package.json",
				"/repos/some-app/packages/faker/package.json",
				"/repos/some-app/packages/left-pad/package.json",
				"/repos/some-app/packages/xzibit/package.json",
				"/repos/some-app/packages/xzibit/packages/yo-dawg/package.json",
			},
		},
		{
			name: "output globbing grabs the desired content",
			files: []string{
				"/external/file.txt",
				"/repos/some-app/src/index.js",
				"/repos/some-app/public/src/css/index.css",
				"/repos/some-app/.turbo/turbo-build.log",
				"/repos/some-app/.turbo/somebody-touched-this-file-into-existence.txt",
				"/repos/some-app/.next/log.txt",
				"/repos/some-app/.next/cache/db6a76a62043520e7aaadd0bb2104e78.txt",
				"/repos/some-app/dist/index.html",
				"/repos/some-app/dist/js/index.js",
				"/repos/some-app/dist/js/lib.js",
				"/repos/some-app/dist/js/node_modules/browserify.js",
				"/repos/some-app/public/dist/css/index.css",
				"/repos/some-app/public/dist/images/rick_astley.jpg",
			},
			args: args{
				basePath:        "/repos/some-app/",
				includePatterns: []string{".turbo/turbo-build.log", "dist/**", ".next/**", "public/dist/**"},
				excludePatterns: []string{},
			},
			wantAll: []string{
				"/repos/some-app/.next",
				"/repos/some-app/.next/cache",
				"/repos/some-app/.next/cache/db6a76a62043520e7aaadd0bb2104e78.txt",
				"/repos/some-app/.next/log.txt",
				"/repos/some-app/.turbo/turbo-build.log",
				"/repos/some-app/dist",
				"/repos/some-app/dist/index.html",
				"/repos/some-app/dist/js",
				"/repos/some-app/dist/js/index.js",
				"/repos/some-app/dist/js/lib.js",
				"/repos/some-app/dist/js/node_modules",
				"/repos/some-app/dist/js/node_modules/browserify.js",
				"/repos/some-app/public/dist",
				"/repos/some-app/public/dist/css",
				"/repos/some-app/public/dist/css/index.css",
				"/repos/some-app/public/dist/images",
				"/repos/some-app/public/dist/images/rick_astley.jpg",
			},
			wantFiles: []string{
				"/repos/some-app/.next/cache/db6a76a62043520e7aaadd0bb2104e78.txt",
				"/repos/some-app/.next/log.txt",
				"/repos/some-app/.turbo/turbo-build.log",
				"/repos/some-app/dist/index.html",
				"/repos/some-app/dist/js/index.js",
				"/repos/some-app/dist/js/lib.js",
				"/repos/some-app/dist/js/node_modules/browserify.js",
				"/repos/some-app/public/dist/css/index.css",
				"/repos/some-app/public/dist/images/rick_astley.jpg",
			},
		},
		{
			name: "passing ** captures all children",
			files: []string{
				"/repos/some-app/dist/index.html",
				"/repos/some-app/dist/js/index.js",
				"/repos/some-app/dist/js/lib.js",
				"/repos/some-app/dist/js/node_modules/browserify.js",
			},
			args: args{
				basePath:        "/repos/some-app/",
				includePatterns: []string{"dist/**"},
				excludePatterns: []string{},
			},
			wantAll: []string{
				"/repos/some-app/dist",
				"/repos/some-app/dist/index.html",
				"/repos/some-app/dist/js",
				"/repos/some-app/dist/js/index.js",
				"/repos/some-app/dist/js/lib.js",
				"/repos/some-app/dist/js/node_modules",
				"/repos/some-app/dist/js/node_modules/browserify.js",
			},
			wantFiles: []string{
				"/repos/some-app/dist/index.html",
				"/repos/some-app/dist/js/index.js",
				"/repos/some-app/dist/js/lib.js",
				"/repos/some-app/dist/js/node_modules/browserify.js",
			},
		},
		{
			name: "passing just a directory captures no children",
			files: []string{
				"/repos/some-app/dist/index.html",
				"/repos/some-app/dist/js/index.js",
				"/repos/some-app/dist/js/lib.js",
				"/repos/some-app/dist/js/node_modules/browserify.js",
			},
			args: args{
				basePath:        "/repos/some-app/",
				includePatterns: []string{"dist"},
				excludePatterns: []string{},
			},
			wantAll:   []string{"/repos/some-app/dist"},
			wantFiles: []string{},
		},
		{
			name: "redundant includes do not duplicate",
			files: []string{
				"/repos/some-app/dist/index.html",
				"/repos/some-app/dist/js/index.js",
				"/repos/some-app/dist/js/lib.js",
				"/repos/some-app/dist/js/node_modules/browserify.js",
			},
			args: args{
				basePath:        "/repos/some-app/",
				includePatterns: []string{"**/*", "dist/**"},
				excludePatterns: []string{},
			},
			wantAll: []string{
				"/repos/some-app/dist",
				"/repos/some-app/dist/index.html",
				"/repos/some-app/dist/js",
				"/repos/some-app/dist/js/index.js",
				"/repos/some-app/dist/js/lib.js",
				"/repos/some-app/dist/js/node_modules",
				"/repos/some-app/dist/js/node_modules/browserify.js",
			},
			wantFiles: []string{
				"/repos/some-app/dist/index.html",
				"/repos/some-app/dist/js/index.js",
				"/repos/some-app/dist/js/lib.js",
				"/repos/some-app/dist/js/node_modules/browserify.js",
			},
		},
		{
			name: "exclude everything, include everything",
			files: []string{
				"/repos/some-app/dist/index.html",
				"/repos/some-app/dist/js/index.js",
				"/repos/some-app/dist/js/lib.js",
				"/repos/some-app/dist/js/node_modules/browserify.js",
			},
			args: args{
				basePath:        "/repos/some-app/",
				includePatterns: []string{"**"},
				excludePatterns: []string{"**"},
			},
			wantAll:   []string{},
			wantFiles: []string{},
		},
		{
			name: "passing just a directory to exclude prevents capture of children",
			files: []string{
				"/repos/some-app/dist/index.html",
				"/repos/some-app/dist/js/index.js",
				"/repos/some-app/dist/js/lib.js",
				"/repos/some-app/dist/js/node_modules/browserify.js",
			},
			args: args{
				basePath:        "/repos/some-app/",
				includePatterns: []string{"dist/**"},
				excludePatterns: []string{"dist/js"},
			},
			wantAll: []string{
				"/repos/some-app/dist",
				"/repos/some-app/dist/index.html",
			},
			wantFiles: []string{
				"/repos/some-app/dist/index.html",
			},
		},
		{
			name: "passing ** to exclude prevents capture of children",
			files: []string{
				"/repos/some-app/dist/index.html",
				"/repos/some-app/dist/js/index.js",
				"/repos/some-app/dist/js/lib.js",
				"/repos/some-app/dist/js/node_modules/browserify.js",
			},
			args: args{
				basePath:        "/repos/some-app/",
				includePatterns: []string{"dist/**"},
				excludePatterns: []string{"dist/js/**"},
			},
			wantAll: []string{
				"/repos/some-app/dist",
				"/repos/some-app/dist/index.html",
				"/repos/some-app/dist/js",
			},
			wantFiles: []string{
				"/repos/some-app/dist/index.html",
			},
		},
		{
			name: "exclude everything with folder . applies at base path",
			files: []string{
				"/repos/some-app/dist/index.html",
				"/repos/some-app/dist/js/index.js",
				"/repos/some-app/dist/js/lib.js",
				"/repos/some-app/dist/js/node_modules/browserify.js",
			},
			args: args{
				basePath:        "/repos/some-app/",
				includePatterns: []string{"**"},
				excludePatterns: []string{"./"},
			},
			wantAll:   []string{},
			wantFiles: []string{},
		},
		{
			name: "exclude everything with traversal applies at a non-base path",
			files: []string{
				"/repos/some-app/dist/index.html",
				"/repos/some-app/dist/js/index.js",
				"/repos/some-app/dist/js/lib.js",
				"/repos/some-app/dist/js/node_modules/browserify.js",
			},
			args: args{
				basePath:        "/repos/some-app/",
				includePatterns: []string{"**"},
				excludePatterns: []string{"./dist"},
			},
			wantAll:   []string{},
			wantFiles: []string{},
		},
		{
			name: "exclude everything with folder traversal (..) applies at base path",
			files: []string{
				"/repos/some-app/dist/index.html",
				"/repos/some-app/dist/js/index.js",
				"/repos/some-app/dist/js/lib.js",
				"/repos/some-app/dist/js/node_modules/browserify.js",
			},
			args: args{
				basePath:        "/repos/some-app/",
				includePatterns: []string{"**"},
				excludePatterns: []string{"dist/../"},
			},
			wantAll:   []string{},
			wantFiles: []string{},
		},
		{
			name: "how do globs even work bad glob microformat",
			files: []string{
				"/repos/some-app/dist/index.html",
				"/repos/some-app/dist/js/index.js",
				"/repos/some-app/dist/js/lib.js",
				"/repos/some-app/dist/js/node_modules/browserify.js",
			},
			args: args{
				basePath:        "/repos/some-app/",
				includePatterns: []string{"**/**/**"},
				excludePatterns: []string{},
			},
			wantAll: []string{
				"/repos/some-app/dist",
				"/repos/some-app/dist/index.html",
				"/repos/some-app/dist/js",
				"/repos/some-app/dist/js/index.js",
				"/repos/some-app/dist/js/lib.js",
				"/repos/some-app/dist/js/node_modules",
				"/repos/some-app/dist/js/node_modules/browserify.js",
			},
			wantFiles: []string{
				"/repos/some-app/dist/index.html",
				"/repos/some-app/dist/js/index.js",
				"/repos/some-app/dist/js/lib.js",
				"/repos/some-app/dist/js/node_modules/browserify.js",
			},
		},
		{
			name: "directory traversal stops at base path",
			files: []string{
				"/repos/spanish-inquisition/index.html",
				"/repos/some-app/dist/index.html",
				"/repos/some-app/dist/js/index.js",
				"/repos/some-app/dist/js/lib.js",
				"/repos/some-app/dist/js/node_modules/browserify.js",
			},
			args: args{
				basePath:        "/repos/some-app/",
				includePatterns: []string{"../spanish-inquisition/**", "dist/**"},
				excludePatterns: []string{},
			},
			wantAll:   []string{},
			wantFiles: []string{},
			wantErr:   true,
		},
		{
			name: "globs and traversal and globs do not cross base path",
			files: []string{
				"/repos/spanish-inquisition/index.html",
				"/repos/some-app/dist/index.html",
				"/repos/some-app/dist/js/index.js",
				"/repos/some-app/dist/js/lib.js",
				"/repos/some-app/dist/js/node_modules/browserify.js",
			},
			args: args{
				basePath:        "/repos/some-app/",
				includePatterns: []string{"**/../../spanish-inquisition/**"},
				excludePatterns: []string{},
			},
			wantAll:   []string{},
			wantFiles: []string{},
			wantErr:   true,
		},
		{
			name: "traversal works within base path",
			files: []string{
				"/repos/some-app/dist/index.html",
				"/repos/some-app/dist/js/index.js",
				"/repos/some-app/dist/js/lib.js",
				"/repos/some-app/dist/js/node_modules/browserify.js",
			},
			args: args{
				basePath:        "/repos/some-app/",
				includePatterns: []string{"dist/js/../**"},
				excludePatterns: []string{},
			},
			wantAll: []string{
				"/repos/some-app/dist",
				"/repos/some-app/dist/index.html",
				"/repos/some-app/dist/js",
				"/repos/some-app/dist/js/index.js",
				"/repos/some-app/dist/js/lib.js",
				"/repos/some-app/dist/js/node_modules",
				"/repos/some-app/dist/js/node_modules/browserify.js",
			},
			wantFiles: []string{
				"/repos/some-app/dist/index.html",
				"/repos/some-app/dist/js/index.js",
				"/repos/some-app/dist/js/lib.js",
				"/repos/some-app/dist/js/node_modules/browserify.js",
			},
		},
		{
			name: "self-references (.) work",
			files: []string{
				"/repos/some-app/dist/index.html",
				"/repos/some-app/dist/js/index.js",
				"/repos/some-app/dist/js/lib.js",
				"/repos/some-app/dist/js/node_modules/browserify.js",
			},
			args: args{
				basePath:        "/repos/some-app/",
				includePatterns: []string{"dist/./././**"},
				excludePatterns: []string{},
			},
			wantAll: []string{
				"/repos/some-app/dist",
				"/repos/some-app/dist/index.html",
				"/repos/some-app/dist/js",
				"/repos/some-app/dist/js/index.js",
				"/repos/some-app/dist/js/lib.js",
				"/repos/some-app/dist/js/node_modules",
				"/repos/some-app/dist/js/node_modules/browserify.js",
			},
			wantFiles: []string{
				"/repos/some-app/dist/index.html",
				"/repos/some-app/dist/js/index.js",
				"/repos/some-app/dist/js/lib.js",
				"/repos/some-app/dist/js/node_modules/browserify.js",
			},
		},
		{
			name: "depth of 1 includes handles folders properly",
			files: []string{
				"/repos/some-app/package.json",
				"/repos/some-app/dist/index.html",
				"/repos/some-app/dist/js/index.js",
				"/repos/some-app/dist/js/lib.js",
				"/repos/some-app/dist/js/node_modules/browserify.js",
			},
			args: args{
				basePath:        "/repos/some-app/",
				includePatterns: []string{"*"},
				excludePatterns: []string{},
			},
			wantAll: []string{
				"/repos/some-app/dist",
				"/repos/some-app/package.json",
			},
			wantFiles: []string{"/repos/some-app/package.json"},
		},
		{
			name: "depth of 1 excludes prevents capturing folders",
			files: []string{
				"/repos/some-app/package.json",
				"/repos/some-app/dist/index.html",
				"/repos/some-app/dist/js/index.js",
				"/repos/some-app/dist/js/lib.js",
				"/repos/some-app/dist/js/node_modules/browserify.js",
			},
			args: args{
				basePath:        "/repos/some-app/",
				includePatterns: []string{"**"},
				excludePatterns: []string{"dist/*"},
			},
			wantAll: []string{
				"/repos/some-app/dist",
				"/repos/some-app/package.json",
			},
			wantFiles: []string{"/repos/some-app/package.json"},
		},
		{
			name: "No-trailing slash basePath works",
			files: []string{
				"/repos/some-app/dist/index.html",
				"/repos/some-app/dist/js/index.js",
				"/repos/some-app/dist/js/lib.js",
				"/repos/some-app/dist/js/node_modules/browserify.js",
			},
			args: args{
				basePath:        "/repos/some-app",
				includePatterns: []string{"dist/**"},
				excludePatterns: []string{},
			},
			wantAll: []string{
				"/repos/some-app/dist",
				"/repos/some-app/dist/index.html",
				"/repos/some-app/dist/js",
				"/repos/some-app/dist/js/index.js",
				"/repos/some-app/dist/js/lib.js",
				"/repos/some-app/dist/js/node_modules",
				"/repos/some-app/dist/js/node_modules/browserify.js",
			},
			wantFiles: []string{
				"/repos/some-app/dist/index.html",
				"/repos/some-app/dist/js/index.js",
				"/repos/some-app/dist/js/lib.js",
				"/repos/some-app/dist/js/node_modules/browserify.js",
			},
		},
		{
			name: "exclude single file",
			files: []string{
				"/repos/some-app/included.txt",
				"/repos/some-app/excluded.txt",
			},
			args: args{
				basePath:        "/repos/some-app",
				includePatterns: []string{"*.txt"},
				excludePatterns: []string{"excluded.txt"},
			},
			wantAll: []string{
				"/repos/some-app/included.txt",
			},
			wantFiles: []string{
				"/repos/some-app/included.txt",
			},
		},
		{
			name: "exclude nested single file",
			files: []string{
				"/repos/some-app/one/included.txt",
				"/repos/some-app/one/two/included.txt",
				"/repos/some-app/one/two/three/included.txt",
				"/repos/some-app/one/excluded.txt",
				"/repos/some-app/one/two/excluded.txt",
				"/repos/some-app/one/two/three/excluded.txt",
			},
			args: args{
				basePath:        "/repos/some-app",
				includePatterns: []string{"**"},
				excludePatterns: []string{"**/excluded.txt"},
			},
			wantAll: []string{
				"/repos/some-app/one/included.txt",
				"/repos/some-app/one/two/included.txt",
				"/repos/some-app/one/two/three/included.txt",
				"/repos/some-app/one",
				"/repos/some-app/one/two",
				"/repos/some-app/one/two/three",
			},
			wantFiles: []string{
				"/repos/some-app/one/included.txt",
				"/repos/some-app/one/two/included.txt",
				"/repos/some-app/one/two/three/included.txt",
			},
		},
		{
			name: "exclude everything",
			files: []string{
				"/repos/some-app/one/included.txt",
				"/repos/some-app/one/two/included.txt",
				"/repos/some-app/one/two/three/included.txt",
				"/repos/some-app/one/excluded.txt",
				"/repos/some-app/one/two/excluded.txt",
				"/repos/some-app/one/two/three/excluded.txt",
			},
			args: args{
				basePath:        "/repos/some-app",
				includePatterns: []string{"**"},
				excludePatterns: []string{"**"},
			},
			wantAll:   []string{},
			wantFiles: []string{},
		},
		{
			name: "exclude everything with slash",
			files: []string{
				"/repos/some-app/one/included.txt",
				"/repos/some-app/one/two/included.txt",
				"/repos/some-app/one/two/three/included.txt",
				"/repos/some-app/one/excluded.txt",
				"/repos/some-app/one/two/excluded.txt",
				"/repos/some-app/one/two/three/excluded.txt",
			},
			args: args{
				basePath:        "/repos/some-app",
				includePatterns: []string{"**"},
				excludePatterns: []string{"**/"},
			},
			wantAll:   []string{},
			wantFiles: []string{},
		},
		{
			name: "exclude everything with leading **",
			files: []string{
				"/repos/some-app/foo/bar",
				"/repos/some-app/some-foo",
				"/repos/some-app/some-foo/bar",
				"/repos/some-app/included",
			},
			args: args{
				basePath:        "/repos/some-app",
				includePatterns: []string{"**"},
				excludePatterns: []string{"**foo"},
			},
			wantAll: []string{
				"/repos/some-app/included",
			},
			wantFiles: []string{
				"/repos/some-app/included",
			},
		},
		{
			name: "exclude everything with trailing **",
			files: []string{
				"/repos/some-app/foo/bar",
				"/repos/some-app/foo-file",
				"/repos/some-app/foo-dir/bar",
				"/repos/some-app/included",
			},
			args: args{
				basePath:        "/repos/some-app",
				includePatterns: []string{"**"},
				excludePatterns: []string{"foo**"},
			},
			wantAll: []string{
				"/repos/some-app/included",
			},
			wantFiles: []string{
				"/repos/some-app/included",
			},
		},
	}
	for _, tt := range tests {
		fsysRoot := "/"
		fsys := setup(fsysRoot, tt.files)

		t.Run(tt.name, func(t *testing.T) {
			got, err := globFilesFs(fsys, fsysRoot, tt.args.basePath, tt.args.includePatterns, tt.args.excludePatterns)

			if (err != nil) != tt.wantErr {
				t.Errorf("globFilesFs() error = %v, wantErr %v", err, tt.wantErr)
				return
			}

			gotToSlash := make([]string, len(got))
			for index, path := range got {
				gotToSlash[index] = filepath.ToSlash(path)
			}

			sort.Strings(gotToSlash)

			if !reflect.DeepEqual(gotToSlash, tt.wantFiles) {
				t.Errorf("globFilesFs() = %v, want %v", gotToSlash, tt.wantFiles)
			}
		})

		t.Run(tt.name, func(t *testing.T) {
			got, err := globAllFs(fsys, fsysRoot, tt.args.basePath, tt.args.includePatterns, tt.args.excludePatterns)

			if (err != nil) != tt.wantErr {
				t.Errorf("globAllFs() error = %v, wantErr %v", err, tt.wantErr)
				return
			}

			gotToSlash := make([]string, len(got))
			for index, path := range got {
				gotToSlash[index] = filepath.ToSlash(path)
			}

			sort.Strings(gotToSlash)
			sort.Strings(tt.wantAll)

			if !reflect.DeepEqual(gotToSlash, tt.wantAll) {
				t.Errorf("globAllFs() = %v, want %v", gotToSlash, tt.wantAll)
			}
		})
	}
}
